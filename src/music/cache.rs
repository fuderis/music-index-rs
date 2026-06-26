use super::{Album, Band, Track};
use crate::prelude::*;

use lofty::{prelude::TaggedFileExt, probe::Probe, tag::Accessor};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::fs;

/// The music cache manager
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct MusicCache {
    /// The main root path of the library (for example, ~/Music)
    pub root_path: Option<PathBuf>,
    /// A map of directory paths and their modification time (mtime)
    pub dirs: HashMap<String, u64>,
    /// Aggregated structure of artists, albums, and tracks
    pub bands: Vec<Band>,
}

impl MusicCache {
    /// Loads the cache from a JSON file
    pub async fn load_from<P: AsRef<Path>>(cache_path: P) -> Result<Self> {
        let data = fs::read_to_string(cache_path).await?;
        let cache = serde_json::from_str::<Self>(&data)?;
        Ok(cache)
    }

    /// Saves the cache to a JSON file
    pub async fn save_to<P: AsRef<Path>>(&self, cache_path: P) -> Result<()> {
        let cache_file = cache_path.as_ref();
        if let Some(parent) = cache_file.parent() {
            fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(cache_file, json).await?;
        Ok(())
    }

    /// Incremental directory scanning
    pub async fn scan_dir<P: AsRef<Path>>(&mut self, root_path: P) -> Result<()> {
        let root = root_path.as_ref();
        let current_dirs = Self::collect_dir_mtimes(root).await?;

        // root switch or first run:
        let root_changed = self.root_path.as_ref() != Some(&root.to_path_buf());
        if root_changed {
            self.root_path = Some(root.to_path_buf());
        }

        // 1. calculating the Diff (deleted, modified, and new folders):
        let mut deleted_dirs = HashSet::new();
        let mut updated_dirs = HashSet::new();

        if root_changed {
            // if the root has changed, we update absolutely everything:
            updated_dirs = current_dirs.keys().map(PathBuf::from).collect();
            self.bands.clear();
        } else {
            // find the deleted directories:
            for old_dir in self.dirs.keys() {
                if !current_dirs.contains_key(old_dir) {
                    deleted_dirs.insert(old_dir.clone());
                }
            }
            // find new or changed directories:
            for (dir_str, &current_mtime) in &current_dirs {
                match self.dirs.get(dir_str) {
                    Some(&old_mtime) if old_mtime == current_mtime => {} // Без изменений
                    _ => {
                        updated_dirs.insert(PathBuf::from(dir_str));
                    }
                }
            }
        }

        if deleted_dirs.is_empty() && updated_dirs.is_empty() {
            return Ok(());
        }

        // 2. clearing old data (Eviction) from the Vec<Band> structure:
        self.evict_obsolete_data(&deleted_dirs, &updated_dirs);

        // 3. scan only updated/new directories (without recursion inside, since the flat map):
        let supported_exts = ["mp3", "flac", "wav", "m4a", "ogg", "opus"];
        let mut raw_tracks = Vec::new();

        for dir in updated_dirs {
            let mut entries = match fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if let Ok(ft) = entry.file_type().await {
                    if ft.is_file() {
                        let ext = match path.extension().and_then(|e| e.to_str()) {
                            Some(e) => e.to_lowercase(),
                            None => continue,
                        };
                        if !supported_exts.contains(&ext.as_str()) {
                            continue;
                        }

                        // reading metadata:
                        let path_clone = path.clone();
                        let tags = tokio::task::spawn_blocking(move || {
                            let probe = Probe::open(&path_clone).ok()?;
                            let tagged_file = probe.read().ok()?;
                            let tag = tagged_file
                                .primary_tag()
                                .or_else(|| tagged_file.first_tag())?;
                            Some((
                                tag.artist().map(|s| s.trim().to_string()),
                                tag.album().map(|s| s.trim().to_string()),
                                tag.title().map(|s| s.trim().to_string()),
                                tag.genre().map(|s| s.trim().to_string()),
                            ))
                        })
                        .await?;

                        let (meta_artist, meta_album, meta_title, meta_genre) =
                            tags.unwrap_or((None, None, None, None));

                        // analysis of the structure relative to the root:
                        let relative = path.strip_prefix(root).unwrap_or(&path);
                        let comps: Vec<String> = relative
                            .iter()
                            .filter_map(|c| c.to_str().map(|s| s.to_string()))
                            .collect();

                        let (path_artist, band_dir) = if comps.len() == 3 {
                            (comps[0].clone(), root.join(&comps[0]))
                        } else if comps.len() >= 4 {
                            (comps[1].clone(), root.join(&comps[0]).join(&comps[1]))
                        } else {
                            let artist = path
                                .parent()
                                .and_then(|p| p.file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "Unknown Artist".to_string());
                            let b_dir = path
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or_else(|| root.to_path_buf());
                            (artist, b_dir)
                        };

                        raw_tracks.push((
                            path,
                            meta_artist,
                            meta_album,
                            meta_title,
                            meta_genre,
                            path_artist,
                            band_dir,
                        ));
                    }
                }
            }
        }

        // 4. hard merging of new tracks into the existing data structure:
        for (path, meta_artist, meta_album, meta_title, meta_genre, path_artist, band_dir) in
            raw_tracks
        {
            let is_valid = |s: &str| {
                !s.is_empty() && !matches!(s.to_lowercase().as_str(), "unknown" | "unknown artist")
            };
            let artist_name = match &meta_artist {
                Some(s) if is_valid(s) => s.clone(),
                _ => path_artist,
            };
            let artist_key = prepare_name(&artist_name);

            let band_entry = if let Some(pos) = self.bands.iter().position(|b| b.text == artist_key)
            {
                &mut self.bands[pos]
            } else {
                self.bands.push(Band {
                    dir: band_dir,
                    name: artist_name,
                    text: artist_key,
                    genres: vec![],
                    albums: vec![],
                });
                self.bands.last_mut().unwrap()
            };

            if let Some(g) = meta_genre {
                for part in g.split(',') {
                    let cleaned = part.trim().to_lowercase();
                    if !cleaned.is_empty()
                        && cleaned != "unknown"
                        && !band_entry.genres.contains(&cleaned)
                    {
                        band_entry.genres.push(cleaned);
                    }
                }
            }

            let album_name = match &meta_album {
                Some(s) if !s.is_empty() && s.to_lowercase() != "unknown" => s.clone(),
                _ => path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Unknown Album".to_string()),
            };
            let album_key = prepare_name(&album_name);

            let album_entry =
                if let Some(pos) = band_entry.albums.iter().position(|a| a.text == album_key) {
                    &mut band_entry.albums[pos]
                } else {
                    band_entry.albums.push(Album {
                        name: album_name,
                        text: album_key,
                        tracks: vec![],
                    });
                    band_entry.albums.last_mut().unwrap()
                };

            let track_name = meta_title.unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
            let track_key = prepare_name(&track_name);

            album_entry.tracks.push(Track {
                name: track_name,
                text: track_key,
                path,
            });
        }

        // 5. final polishing of the structure (sorting):
        for band in &mut self.bands {
            if band.genres.is_empty() {
                band.genres.push("unknown".to_string());
            }
            band.albums.sort_by(|a, b| a.text.cmp(&b.text));
            for album in &mut band.albums {
                album.tracks.sort_by(|a, b| a.path.cmp(&b.path));
            }
        }
        self.bands.sort_by(|a, b| a.text.cmp(&b.text));

        // updating the directory map in the cache state:
        self.dirs = current_dirs;
        Ok(())
    }

    /// Deletes old data from modified or deleted directories from the structure
    fn evict_obsolete_data(
        &mut self,
        deleted_dirs: &HashSet<String>,
        updated_dirs: &HashSet<PathBuf>,
    ) {
        for band in &mut self.bands {
            for album in &mut band.albums {
                album.tracks.retain(|track| {
                    if let Some(parent) = track.path.parent() {
                        if updated_dirs.contains(parent) {
                            return false; // it will be re-scanned
                        }
                    }
                    // if any ancestor of the path is in deleted folders:
                    for ancestor in track.path.ancestors() {
                        if deleted_dirs.contains(&ancestor.to_string_lossy().to_string()) {
                            return false;
                        }
                    }
                    true
                });
            }
            band.albums.retain(|album| !album.tracks.is_empty());
        }
        self.bands.retain(|band| !band.albums.is_empty());
    }

    /// Collecting the mtime of all folders recursively (algorithmically optimized via stack)
    async fn collect_dir_mtimes(root: &Path) -> Result<HashMap<String, u64>> {
        let mut mtimes = HashMap::new();
        let mut stack = vec![root.to_path_buf()];

        while let Some(dir) = stack.pop() {
            if let Ok(meta) = fs::metadata(&dir).await {
                if let Ok(modified) = meta.modified() {
                    if let Ok(dur) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                        if let Some(path_str) = dir.to_str() {
                            mtimes.insert(path_str.to_owned(), dur.as_secs());
                        }
                    }
                }
            }

            if let Ok(mut entries) = fs::read_dir(&dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Ok(ft) = entry.file_type().await {
                        if ft.is_dir() {
                            stack.push(entry.path());
                        }
                    }
                }
            }
        }
        Ok(mtimes)
    }
}

/// Prepares a name optimized to search
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
