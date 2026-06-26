use super::Album;
use crate::prelude::*;

/// The music track structure
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Band {
    pub dir: PathBuf,
    pub name: String,
    pub text: String,
    pub genres: Vec<String>,
    pub albums: Vec<Album>,
}
