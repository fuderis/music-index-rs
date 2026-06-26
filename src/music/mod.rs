pub mod track;
pub use track::Track;

pub mod album;
pub use album::Album;

pub mod band;
pub use band::Band;

pub mod cache;
pub use cache::MusicCache;

use crate::prelude::*;
use std::process::Stdio;
use tokio::{fs, process::Command};

const CMP_COEF: f32 = 0.35;

/// The playback target wrapper
#[derive(Debug, Clone)]
pub enum PlaybackTarget<'a> {
    Band(Vec<&'a Band>),
    Album(Vec<&'a Album>),
    Tracks(Vec<&'a Track>),
    None,
}

impl<'a> PlaybackTarget<'a> {
    pub fn tracks(&self) -> Vec<&'a Track> {
        match self {
            PlaybackTarget::None => vec![],

            PlaybackTarget::Tracks(tracks) => tracks.clone(),

            PlaybackTarget::Album(albums) => albums
                .iter()
                .flat_map(|album| album.tracks.iter())
                .collect(),

            PlaybackTarget::Band(bands) => bands
                .iter()
                .flat_map(|band| band.albums.iter())
                .flat_map(|album| album.tracks.iter())
                .collect(),
        }
    }

    pub fn into_tracks(self) -> Vec<&'a Track> {
        match self {
            PlaybackTarget::None => vec![],

            PlaybackTarget::Tracks(tracks) => tracks,

            PlaybackTarget::Album(albums) => albums
                .into_iter()
                .flat_map(|album| album.tracks.iter())
                .collect(),

            PlaybackTarget::Band(bands) => bands
                .into_iter()
                .flat_map(|band| band.albums.iter())
                .flat_map(|album| album.tracks.iter())
                .collect(),
        }
    }
}

/// The music search intent
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub enum SearchIntent {
    Global(String),
    Targeted {
        band: Option<String>,
        album: Option<String>,
        track: Option<String>,
        genre: Option<String>,
    },
}

/// The music search manager
pub struct MusicIndexer {
    pub cache: MusicCache,
}

impl MusicIndexer {
    /// Initializes the search engine, rolls an incremental sync, and saves the cache
    pub async fn scan_dir<P: AsRef<Path>>(root_path: P, cache_path: P) -> Result<Self> {
        let cache_path_buf = cache_path.as_ref().to_path_buf();

        // trying to load cache or create a default new one:
        let mut cache = MusicCache::load_from(&cache_path_buf)
            .await
            .unwrap_or_default();

        // starting scanning the dir:
        cache.scan_dir(root_path).await?;
        cache.save_to(&cache_path_buf).await?;

        Ok(Self { cache })
    }

    /// A quick helper in case you want to run a scan of the default OS folder
    pub async fn scan_default<P: AsRef<Path>>(cache_path: P) -> Result<Self> {
        let default_dir = Self::get_default_music_dir()
            .ok_or_else(|| str!("Couldn't identify the standard music folder"))?;
        Self::scan_dir(default_dir, cache_path.as_ref().to_path_buf()).await
    }

    fn get_default_music_dir() -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Music"))
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Music"))
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        {
            None
        }
    }

    /// Searches a music by query
    pub fn find(&self, query: &str) -> Vec<&Track> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
        if words.is_empty() {
            return vec![];
        }

        let mut results = vec![];
        for band in &self.cache.bands {
            for album in &band.albums {
                for track in &album.tracks {
                    let combined_text = format!("{} {} {}", band.text, album.text, track.text);
                    if words.iter().all(|word| combined_text.contains(word)) {
                        results.push(track);
                    }
                }
            }
        }

        results
    }

    /// Searches a music by genre
    pub fn find_genre(&self, genre: &str) -> Vec<&Band> {
        let genre_lower = genre.to_lowercase();
        self.cache
            .bands
            .iter()
            .filter(|b| b.genres.iter().any(|g| g.to_lowercase() == genre_lower))
            .collect()
    }

    /// Searches a music by band name
    pub fn find_band(&self, band_name: &str) -> Option<&Band> {
        let target = prepare_name(band_name);
        self.cache.bands.iter().find(|b| b.text == target)
    }

    /// Searches a music by album name
    pub fn find_album(&self, band_name: &str) -> Vec<&Album> {
        self.find_band(band_name)
            .map(|b| b.albums.iter().collect())
            .unwrap_or_default()
    }

    /// Searches based by one specific song name
    pub fn find_track(&self, band_name: &str, album_name: Option<&str>) -> Vec<&Track> {
        let band = match self.find_band(band_name) {
            Some(b) => b,
            None => return vec![],
        };

        if let Some(alb_name) = album_name {
            let alb_target = prepare_name(alb_name);
            band.albums
                .iter()
                .find(|a| a.text == alb_target)
                .map(|a| a.tracks.iter().collect())
                .unwrap_or_default()
        } else {
            band.albums.iter().flat_map(|a| a.tracks.iter()).collect()
        }
    }

    /// The entry point for all core search strategies
    pub fn search(&self, intent: SearchIntent) -> PlaybackTarget<'_> {
        match intent {
            SearchIntent::Global(query) => self.perform_global_cascade_search(query),
            SearchIntent::Targeted {
                band,
                album,
                track,
                genre,
            } => self.perform_combined_search(band, album, track, genre),
        }
    }

    /// Cascading global search: Bands -> Albums -> Tracks
    fn perform_global_cascade_search(&self, query: String) -> PlaybackTarget<'_> {
        let cleaned = prepare_name(&query);

        // =========================
        // TIER 1: BANDS
        // =========================
        let mut bands: Vec<(f32, &Band)> = Vec::new();

        for band in &self.cache.bands {
            let score = fuzzy_cmp::hybrid_compare(&band.text, &cleaned);

            if score >= CMP_COEF {
                bands.push((score, band));
            }
        }

        bands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

        if !bands.is_empty() {
            let max = bands.first().unwrap().0;
            let mean = bands.iter().map(|x| x.0).sum::<f32>() / bands.len() as f32;

            let threshold = max * 0.7 + mean * 0.3;

            let filtered: Vec<&Band> = bands
                .into_iter()
                .filter(|(score, _)| *score >= threshold)
                .map(|(_, b)| b)
                .collect();

            if !filtered.is_empty() {
                return PlaybackTarget::Band(filtered);
            }
        }

        // =========================
        // TIER 2: ALBUMS
        // =========================
        let mut albums: Vec<(f32, &Album)> = Vec::new();

        for band in &self.cache.bands {
            for album in &band.albums {
                let score = fuzzy_cmp::hybrid_compare(&album.text, &cleaned);

                if score >= CMP_COEF {
                    albums.push((score, album));
                }
            }
        }

        albums.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

        if !albums.is_empty() {
            let max = albums.first().unwrap().0;
            let mean = albums.iter().map(|x| x.0).sum::<f32>() / albums.len() as f32;

            let threshold = max * 0.7 + mean * 0.3;

            let filtered: Vec<&Album> = albums
                .into_iter()
                .filter(|(score, _)| *score >= threshold)
                .map(|(_, a)| a)
                .collect();

            if !filtered.is_empty() {
                return PlaybackTarget::Album(filtered);
            }
        }

        // =========================
        // TIER 3: TRACKS
        // =========================
        let mut tracks: Vec<(f32, &Track)> = Vec::new();

        for band in &self.cache.bands {
            for album in &band.albums {
                for track in &album.tracks {
                    let score = fuzzy_cmp::hybrid_compare(&track.text, &cleaned);

                    if score >= CMP_COEF {
                        tracks.push((score, track));
                    }
                }
            }
        }

        if tracks.is_empty() {
            return PlaybackTarget::None;
        }

        tracks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

        let max = tracks.first().unwrap().0;
        let mean = tracks.iter().map(|x| x.0).sum::<f32>() / tracks.len() as f32;

        let threshold = max * 0.7 + mean * 0.3;

        let filtered: Vec<&Track> = tracks
            .into_iter()
            .filter(|(score, _)| *score >= threshold)
            .map(|(_, t)| t)
            .collect();

        if filtered.is_empty() {
            PlaybackTarget::None
        } else {
            PlaybackTarget::Tracks(filtered)
        }
    }

    /// Search with intersection of specific structural filters
    fn perform_combined_search(
        &self,
        band_opt: Option<String>,
        album_opt: Option<String>,
        track_opt: Option<String>,
        genre_opt: Option<String>,
    ) -> PlaybackTarget<'_> {
        let band_filter = band_opt.map(|s| prepare_name(&s));
        let album_filter = album_opt.map(|s| prepare_name(&s));
        let track_filter = track_opt.map(|s| prepare_name(&s));
        let genre_filter = genre_opt.map(|s| prepare_name(&s));

        let has_band = band_filter.is_some();
        let has_album = album_filter.is_some();
        let has_track = track_filter.is_some();
        let has_genre = genre_filter.is_some();

        // =========================
        // STRATEGY 1: BAND ONLY
        // =========================
        if has_band && !has_album && !has_track && !has_genre {
            let mut bands: Vec<(f32, &Band)> = Vec::new();

            let b_ref = band_filter.as_ref().unwrap();

            for band in &self.cache.bands {
                let score = fuzzy_cmp::hybrid_compare(&band.text, b_ref);

                if score >= CMP_COEF {
                    bands.push((score, band));
                }
            }

            if bands.is_empty() {
                return PlaybackTarget::None;
            }

            bands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

            let max = bands.first().unwrap().0;
            let mean = bands.iter().map(|x| x.0).sum::<f32>() / bands.len() as f32;

            let threshold = max * 0.7 + mean * 0.3;

            let filtered: Vec<&Band> = bands
                .into_iter()
                .filter(|(score, _)| *score >= threshold)
                .map(|(_, b)| b)
                .collect();

            return if filtered.is_empty() {
                PlaybackTarget::None
            } else {
                PlaybackTarget::Band(filtered)
            };
        }

        // =========================
        // STRATEGY 2: ALBUM
        // =========================
        if has_album && !has_track && !has_genre {
            let mut albums: Vec<(f32, &Album)> = Vec::new();

            for band in &self.cache.bands {
                let band_score = if let Some(ref b_ref) = band_filter {
                    let score = fuzzy_cmp::hybrid_compare(&band.text, b_ref);
                    if score < CMP_COEF {
                        continue;
                    }
                    score
                } else {
                    1.0
                };

                for album in &band.albums {
                    let album_score = if let Some(ref a_ref) = album_filter {
                        let score = fuzzy_cmp::hybrid_compare(&album.text, a_ref);
                        if score < CMP_COEF {
                            continue;
                        }
                        score
                    } else {
                        1.0
                    };

                    albums.push((band_score * album_score, album));
                }
            }

            if albums.is_empty() {
                return PlaybackTarget::None;
            }

            albums.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

            let max = albums.first().unwrap().0;
            let mean = albums.iter().map(|x| x.0).sum::<f32>() / albums.len() as f32;

            let threshold = max * 0.7 + mean * 0.3;

            let filtered: Vec<&Album> = albums
                .into_iter()
                .filter(|(score, _)| *score >= threshold)
                .map(|(_, a)| a)
                .collect();

            return if filtered.is_empty() {
                PlaybackTarget::None
            } else {
                PlaybackTarget::Album(filtered)
            };
        }

        // =========================
        // STRATEGY 3: TRACKS
        // =========================
        let mut tracks: Vec<(f32, &Track)> = Vec::new();

        for band in &self.cache.bands {
            if let Some(ref g_ref) = genre_filter {
                if !band
                    .genres
                    .iter()
                    .any(|g| fuzzy_cmp::hybrid_compare(g, g_ref) >= CMP_COEF)
                {
                    continue;
                }
            }

            let band_score = if let Some(ref b_ref) = band_filter {
                let score = fuzzy_cmp::hybrid_compare(&band.text, b_ref);
                if score < CMP_COEF {
                    continue;
                }
                score
            } else {
                1.0
            };

            for album in &band.albums {
                let album_score = if let Some(ref a_ref) = album_filter {
                    let score = fuzzy_cmp::hybrid_compare(&album.text, a_ref);
                    if score < CMP_COEF {
                        continue;
                    }
                    score
                } else {
                    1.0
                };

                for track in &album.tracks {
                    let track_score = if let Some(ref t_ref) = track_filter {
                        let score = fuzzy_cmp::hybrid_compare(&track.text, t_ref);
                        if score < CMP_COEF {
                            continue;
                        }
                        score
                    } else {
                        1.0
                    };

                    let total_score = band_score * album_score * track_score;

                    tracks.push((total_score, track));
                }
            }
        }

        if tracks.is_empty() {
            return PlaybackTarget::None;
        }

        tracks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

        let max = tracks.first().unwrap().0;
        let mean = tracks.iter().map(|x| x.0).sum::<f32>() / tracks.len() as f32;

        let threshold = max * 0.7 + mean * 0.3;

        let filtered: Vec<&Track> = tracks
            .into_iter()
            .filter(|(score, _)| *score >= threshold)
            .map(|(_, t)| t)
            .collect();

        if filtered.is_empty() {
            PlaybackTarget::None
        } else {
            PlaybackTarget::Tracks(filtered)
        }
    }

    /// Starting playback of multiple paths (folders or specific files) in a single playlist
    pub async fn play(&self, target: PlaybackTarget<'_>, playlist: impl AsRef<Path>) -> Result<()> {
        let playlist = playlist.as_ref();

        self.create_playlist(target, playlist).await?;
        self.launch_player(playlist).await?;

        Ok(())
    }

    /// Creates the M3U playlist file
    pub async fn create_playlist<'a>(
        &self,
        target: PlaybackTarget<'a>,
        playlist: impl AsRef<Path>,
    ) -> Result<()> {
        let mut songs_list: Vec<&Path> = target
            .into_tracks()
            .into_iter()
            .map(|track| track.path.as_path())
            .collect();

        songs_list.sort();

        let mut content = Vec::new();
        content.extend_from_slice(b"#EXTM3U\n");

        for song in songs_list {
            let unix_path = song.to_string_lossy().replace('\\', "/");

            if let Some(filename) = song.file_name().and_then(|f| f.to_str()) {
                content.extend_from_slice(
                    format!("#EXTINF:-1,{}\n{}\n", filename, unix_path).as_bytes(),
                );
            }
        }

        fs::write(playlist.as_ref(), content).await?;

        Ok(())
    }

    /// Launches the playlist in program by default
    pub async fn launch_player(&self, playlist_file: &Path) -> Result<()> {
        let playlist_str = playlist_file.to_string_lossy();

        #[cfg(unix)]
        {
            Command::new("xdg-open")
                .arg(playlist_str.as_ref())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        }

        #[cfg(windows)]
        {
            Command::new("cmd")
                .args(["/C", "start", "", &playlist_str])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        }

        Ok(())
    }
}

/// Prepares the query string before searching
fn prepare_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
