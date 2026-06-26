#![allow(unused_imports)]
use crate::prelude::DynError;
use macron::{Display, Error, From};

/// The error instance
#[derive(Debug, Display, Error, From)]
pub enum Error {
    Io(std::io::Error),

    #[display(fmt = "Unsupported operating system")]
    UnsupportedOS,
}
