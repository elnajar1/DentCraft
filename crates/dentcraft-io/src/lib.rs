//! Mesh import and export.

mod obj;
mod ply;
mod stl;

use std::path::{Path, PathBuf};

use dentcraft_mesh::TriMesh;

pub use obj::{read_obj, write_obj};
pub use ply::{read_ply, write_ply_binary};
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
    #[error("{path}: unsupported file type (expected .stl, .ply or .obj)")]
    Unsupported { path: PathBuf },
}

/// File formats DentCraft reads and writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Stl,
    Ply,
    Obj,
}

impl Format {
    /// Chooses the format from the file extension, ignoring case.
    pub fn from_path(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "stl" => Some(Format::Stl),
            "ply" => Some(Format::Ply),
            "obj" => Some(Format::Obj),
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
        Format::Stl => read_stl(&bytes),
        Format::Ply => read_ply(&bytes),
        Format::Obj => read_obj(&bytes),
    }
    .map_err(malformed)
}

/// Writes a mesh as binary STL, binary PLY or OBJ, chosen from the file extension.
pub fn save_mesh(path: &Path, mesh: &TriMesh) -> Result<(), IoError> {
    let format =
        Format::from_path(path).ok_or_else(|| IoError::Unsupported { path: path.into() })?;
    let bytes = match format {
        Format::Stl => write_stl_binary(mesh),
        Format::Ply => write_ply_binary(mesh),
        Format::Obj => write_obj(mesh).into_bytes(),
    };
    std::fs::write(path, bytes).map_err(|source| IoError::Io {
        path: path.into(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn format_follows_extension_ignoring_case() {
        for (name, format) in [
            ("a.STL", Some(Format::Stl)),
            ("a.Ply", Some(Format::Ply)),
            ("dir/a.obj", Some(Format::Obj)),
            ("a.txt", None),
            ("a", None),
        ] {
            assert_eq!(Format::from_path(Path::new(name)), format, "{name}");
        }
    }

    #[test]
    fn save_and_load_round_trip_each_format() {
        let dir = std::env::temp_dir().join(format!("dentcraft-io-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mesh = TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z],
            vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]],
        );
        for name in ["t.stl", "t.PLY", "t.obj"] {
            let path = dir.join(name);
            save_mesh(&path, &mesh).unwrap();
            let back = load_mesh(&path).unwrap();
            assert_eq!(
                (back.vertex_count(), back.triangle_count()),
                (4, 4),
                "{name}"
            );
        }
        let err = save_mesh(&dir.join("t.txt"), &mesh).unwrap_err();
        assert!(err.to_string().contains("expected .stl, .ply or .obj"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
