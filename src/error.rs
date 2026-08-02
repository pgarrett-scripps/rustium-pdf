//! Error type. One enum, no dependency on an error-derive crate.

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// The file could not be read from disk.
    Io(std::io::Error),
    /// The file is not a PDF or its cross-reference structure is beyond repair.
    Parse(String),
    /// The document is encrypted and the supplied password does not open it.
    Password,
    /// The document uses an encryption scheme this crate does not implement.
    UnsupportedEncryption(String),
    /// A page index past the end of the document.
    PageOutOfRange(usize),
    /// A page exists but its content could not be processed.
    Page { index: usize, message: String },
    /// Rendering failed (allocation, encoding).
    Render(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Parse(m) => write!(f, "malformed pdf: {m}"),
            Error::Password => write!(f, "invalid password"),
            Error::UnsupportedEncryption(m) => write!(f, "unsupported encryption: {m}"),
            Error::PageOutOfRange(i) => write!(f, "page index {i} out of range"),
            Error::Page { index, message } => write!(f, "page {index}: {message}"),
            Error::Render(m) => write!(f, "render error: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
