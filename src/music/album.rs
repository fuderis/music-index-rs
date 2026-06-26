use super::Track;
use crate::prelude::*;

/// The music track structure
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Album {
    pub name: String,
    pub text: String,
    pub tracks: Vec<Track>,
}
