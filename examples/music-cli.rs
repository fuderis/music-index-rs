#![cfg(feature = "test-cli")]
use music_index::{MusicIndexer, PlaybackTarget, SearchIntent};

use clap::Parser;
use macron::path;
use std::path::PathBuf;
use tokio::time::Instant;

#[derive(Parser, Debug)]
#[command(
    name = "muson",
    author = "Fuderis",
    version = "0.1.0",
    about = "Smart music player with combined search strategies",
    arg_required_else_help = true
)]
struct Args {
    /// Global smart full-text search (conflicts with targeted filters)
    #[arg(
        short,
        long,
        value_name = "QUERY",
        conflicts_with_all = ["band", "album", "track", "genre"]
    )]
    search: Option<String>,

    /// Filter specifically by band / artist name
    #[arg(short, long, value_name = "NAME")]
    band: Option<String>,

    /// Filter specifically by album title
    #[arg(short, long, value_name = "TITLE")]
    album: Option<String>,

    /// Filter specifically by track name
    #[arg(short, long, value_name = "NAME")]
    track: Option<String>,

    /// Filter specifically by genre
    #[arg(short, long, value_name = "GENRE")]
    genre: Option<String>,

    /// Trigger actual playback (if omitted, only lists matches)
    #[arg(short, long)]
    play: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let start_time = Instant::now();

    let music_index = MusicIndexer::scan_dir(path!("~/Music"), path!("~/music.cache")).await?;
    let intent = if let Some(ref query) = args.search {
        SearchIntent::Global { query }
    } else {
        SearchIntent::Targeted {
            band: args.band.as_deref(),
            album: args.album.as_deref(),
            track: args.track.as_deref(),
            genre: args.genre.as_deref(),
        }
    };

    let search_time = Instant::now();
    let target = music_index.search(intent);
    let search_took = search_time.elapsed();

    if args.play {
        play_target(&music_index, target).await?;
    } else {
        print_playback_contents(&target);
    }

    println!(
        "⏱ Search took {:?}\n⏱ Total time {:?}",
        search_took,
        start_time.elapsed()
    );

    Ok(())
}

async fn play_target(
    music: &MusicIndexer,
    target: PlaybackTarget<'_>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match &target {
        PlaybackTarget::None => {
            println!("✖ Nothing found");
            return Ok(());
        }

        PlaybackTarget::Band(_) => {
            println!("▶ Playing full artist selection");
        }

        PlaybackTarget::Album(_) => {
            println!("▶ Playing album selection");
        }

        PlaybackTarget::Tracks(tracks) => {
            println!("▶ Playing {} track(s)", tracks.len());
        }
    }

    print_playback_contents(&target);

    music.play(target, playlist_path()).await?;
    Ok(())
}

fn print_playback_contents(target: &PlaybackTarget) {
    match target {
        PlaybackTarget::None => {
            println!("No playable content found.");
        }

        PlaybackTarget::Tracks(tracks) => {
            println!("▶ Tracks to be played ({}):", tracks.len());
            for t in tracks {
                println!("  🎵 {}", t.name);
            }
        }

        PlaybackTarget::Album(albums) => {
            let mut count = 0;

            for album in albums {
                for track in &album.tracks {
                    println!("  💿 {} — 🎵 {}", album.name, track.name);
                    count += 1;
                }
            }

            println!("\n▶ Total tracks to be played: {}", count);
        }

        PlaybackTarget::Band(bands) => {
            let mut count = 0;

            for band in bands {
                for album in &band.albums {
                    for track in &album.tracks {
                        println!("  🎸 {} / 💿 {} — 🎵 {}", band.name, album.name, track.name);
                        count += 1;
                    }
                }
            }

            println!("\n▶ Total tracks to be played: {}", count);
        }
    }
}

fn playlist_path() -> PathBuf {
    std::env::temp_dir().join("playlist.m3u")
}
