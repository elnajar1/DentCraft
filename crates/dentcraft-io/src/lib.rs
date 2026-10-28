//! Mesh import and export.

mod stl;

use std::path::{Path, PathBuf};

use dentcraft_mesh::TriMesh;

pub use stl::{read_stl, write_stl_binary};

#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("cannot read or write {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Malformed { path: PathBuf, message: String },
    #[error("{path}: unsupported file type (expected .stl)")]
    Unsupported { path: PathBuf },
}

/// File formats DentCraft reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Stl,
}

impl Format {
    pub fn from_path(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "stl" => Some(Format::Stl),
            _ => None,
        }
    }
}

/// Reads a mesh, choosing the format from the file extension.
pub fn load_mesh(path: &Path) -> Result<TriMesh, IoError> {
    let format =
        Format::from_path(path).ok_or_else(|| IoError::Unsupported { path: path.into() })?;
    let bytes = std::fs::read(path).map_err(|source| IoError::Io {
        path: path.into(),
        source,
    })?;
    let malformed = |message: String| IoError::Malformed {
        path: path.into(),
        message,
    };
    match format {
        Format::Stl => read_stl(&bytes).map_err(malformed),
    }
}
