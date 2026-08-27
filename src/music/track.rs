use crate::prelude::*;

/// The music track structure
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Track {
    pub band: String,
    pub name: String,
    pub text: String,
    pub path: PathBuf,
}
