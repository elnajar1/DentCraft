//! Background loading of scan files: parse, prepare render data and build the picking BVH off
//! the UI thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use dentcraft_mesh::{Bvh, TriMesh};
use dentcraft_render::{MeshData, MeshId};

use crate::crash::{LOAD_PREFIX, PANIC_MESSAGE, spawn_worker};

/// A scan ready to show and pick.
pub struct Loaded {
    pub mesh: TriMesh,
    pub bvh: Bvh,
    /// Vertex data for the GPU, shared with the paint callback.
    pub render: Arc<MeshData>,
}

/// Outcome of one load, sent back to the UI thread.
pub struct LoadMessage {
    pub id: MeshId,
    pub path: PathBuf,
    pub result: Result<Loaded, String>,
}

/// Spawns one worker thread per file and collects their results.
pub struct Loader {
    tx: Sender<LoadMessage>,
    rx: Receiver<LoadMessage>,
    /// Woken when a result arrives, so the UI picks it up without polling.
    repaint: Option<eframe::egui::Context>,
}

impl Loader {
    pub fn new(repaint: Option<eframe::egui::Context>) -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, repaint }
    }

    pub fn spawn(&self, id: MeshId, path: PathBuf) {
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        let name = format!("{LOAD_PREFIX}{}", path.display());
        let source = path.clone();
        let deliver = move |result: Result<Result<Loaded, String>, String>| {
            let result = result.and_then(|r| r).map_err(|e| {
                if e == PANIC_MESSAGE {
                    "the loader crashed on this file. Details were saved to the crash log."
                        .to_owned()
                } else {
                    e
                }
            });
            // The receiver is gone only when the app is shutting down.
            let _ = tx.send(LoadMessage { id, path, result });
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        };
        spawn_worker(&name, move || load(&source), deliver);
    }

    /// Results that arrived since the last call.
    pub fn poll(&self) -> Vec<LoadMessage> {
        self.rx.try_iter().collect()
    }
}

fn load(path: &std::path::Path) -> Result<Loaded, String> {
    let mesh = dentcraft_io::load_mesh(path).map_err(|e| e.to_string())?;
    if mesh.triangle_count() == 0 {
        return Err(format!(
            "{}: the file contains no triangles",
            path.display()
        ));
    }
    let render = Arc::new(MeshData::from_mesh(&mesh));
    let bvh = Bvh::build(&mesh);
    Ok(Loaded { mesh, bvh, render })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn wait(loader: &Loader) -> LoadMessage {
        for _ in 0..500 {
            if let Some(msg) = loader.poll().pop() {
                return msg;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("loader did not answer");
    }

    #[test]
    fn loads_a_file_on_a_worker_thread() {
        let mesh = TriMesh::uv_sphere(5.0, 16, 8);
        let path =
            std::env::temp_dir().join(format!("dentcraft-loader-{}.stl", std::process::id()));
        std::fs::write(&path, dentcraft_io::write_stl_binary(&mesh)).unwrap();

        let loader = Loader::new(None);
        loader.spawn(MeshId(7), path.clone());
        let msg = wait(&loader);
        std::fs::remove_file(&path).unwrap();

        assert_eq!(msg.id, MeshId(7));
        let loaded = msg.result.expect("sphere loads");
        assert_eq!(loaded.mesh.triangle_count(), mesh.triangle_count());
        assert!(!loaded.render.indices.is_empty());
        assert!(
            loaded
                .bvh
                .raycast(
                    &loaded.mesh,
                    glam::DVec3::new(0.0, 0.0, 20.0),
                    glam::DVec3::NEG_Z
                )
                .is_some()
        );
    }

    #[test]
    fn a_missing_file_reports_an_error_instead_of_panicking() {
        let loader = Loader::new(None);
        loader.spawn(MeshId(1), PathBuf::from("does/not/exist.stl"));
        let msg = wait(&loader);
        assert!(msg.result.is_err());
    }
}
