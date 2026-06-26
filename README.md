[![github]](https://github.com/fuderis/music-index-rs)&ensp;
[![crates-io]](https://crates.io/crates/music-index)&ensp;
[![docs-rs]](https://docs.rs/music-index)

[github]: https://img.shields.io/badge/github-8da0cb?style=for-the-badge&labelColor=555555&logo=github
[crates-io]: https://img.shields.io/badge/crates.io-fc8d62?style=for-the-badge&labelColor=555555&logo=rust
[docs-rs]: https://img.shields.io/badge/docs.rs-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs

# Music Indexer: A smart music player with combined search strategies

A lightweight Rust library for smart music library indexing, fuzzy search, and playback management.

## Features:

* Recursive music directory scanning
* Cached music index (MusicCache)
* Fuzzy search across: Bands, Albums, Tracks, Genres
* Global + targeted search strategies
* Hierarchical model: Band → Album → Track
* M3U playlist generation
* System player integration (Linux/Windows/macOS)

## Installation:
```bash
cargo add music-index
```

## Examples:

```rust
use music_index::{MusicIndexer, SearchIntent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let music_index = MusicIndexer::scan_dir("~/Music", "~/music.cache").await?;

    let result = music_index.search(SearchIntent::Targeted {
        band: Some("Nirvana"),
        album: Some("Unplugged"),
        track: None,
        genre: None
    });

    for track in result.tracks() {
        println!("🎵 {}", track.name);
    }

    Ok(())
}
```

## Search API:

### Global search:

```rust
SearchIntent::Global { query: "metallica black album" }
```
Cascade order: Bands → Albums → Tracks

### Targeted search:

```rust
SearchIntent::Targeted {
    band: Some("Metallica"),
    album: Some("Master of Puppets"),
    track: None,
    genre: None,
}
```

### Results:

```rust
enum PlaybackTarget {
    Band(Vec<&Band>),
    Album(Vec<&Album>),
    Tracks(Vec<&Track>),
    None,
}

target.tracks();
// or
target.into_tracks();
```

### Playback:

Creates an `M3U` playlist and launches system player:

```rust
music_index.play(target, "playlist.m3u").await?;
```

## License & Feedback:

> This library distributed under the [MIT](https://github.com/fuderis/music-index-rs/blob/main/LICENSE.md) license.

You can contact me via [GitHub](https://github.com/fuderis) or send a message to my [E-Mail](mailto:synapdrake@ya.ru).
This library is actively evolving, and your suggestions and feedback are always welcome!
