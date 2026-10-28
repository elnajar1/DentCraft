//! STL, binary and ASCII. Coincident corners are welded into shared vertices on import.

use std::collections::HashMap;

use dentcraft_mesh::TriMesh;
use glam::DVec3;

/// Parses binary or ASCII STL. Corners with bit-identical coordinates become one vertex.
pub fn read_stl(bytes: &[u8]) -> Result<TriMesh, String> {
    if is_binary(bytes) {
        read_binary(bytes)
    } else {
        read_ascii(bytes)
    }
}

/// Binary STL is 84 header bytes plus 50 per triangle; ASCII files rarely match that exactly.
fn is_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as u64;
    84 + count * 50 == bytes.len() as u64
}

fn read_binary(bytes: &[u8]) -> Result<TriMesh, String> {
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    let mut welder = Welder::with_capacity(count);
    for i in 0..count {
        let record = &bytes[84 + i * 50..84 + (i + 1) * 50];
        let corner = |k: usize| {
            let f = |j: usize| {
                let at = 12 + k * 12 + j * 4;
                f32::from_le_bytes(record[at..at + 4].try_into().unwrap())
            };
            [f(0), f(1), f(2)]
        };
        welder.push_triangle([corner(0), corner(1), corner(2)])?;
    }
    Ok(welder.finish())
}

fn read_ascii(bytes: &[u8]) -> Result<TriMesh, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not a binary STL and not ASCII text")?;
    if !text.trim_start().starts_with("solid") {
        return Err("not an STL file".into());
    }
    let mut welder = Welder::with_capacity(0);
    let mut corners: Vec<[f32; 3]> = Vec::with_capacity(3);
    for (line_no, line) in text.lines().enumerate() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("vertex") => {
                let mut xyz = [0f32; 3];
                for value in &mut xyz {
                    *value = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(|| format!("line {}: bad vertex", line_no + 1))?;
                }
                corners.push(xyz);
            }
            Some("endloop") => {
                let triangle: [[f32; 3]; 3] = corners
                    .as_slice()
                    .try_into()
                    .map_err(|_| format!("line {}: facet without 3 vertices", line_no + 1))?;
                welder.push_triangle(triangle)?;
                corners.clear();
            }
            _ => {}
        }
    }
    Ok(welder.finish())
}

struct Welder {
    index: HashMap<[u32; 3], u32>,
    mesh: TriMesh,
}

impl Welder {
    fn with_capacity(triangles: usize) -> Self {
        Self {
            index: HashMap::with_capacity(triangles / 2),
            mesh: TriMesh {
                positions: Vec::with_capacity(triangles / 2),
                triangles: Vec::with_capacity(triangles),
            },
        }
    }

    fn push_triangle(&mut self, corners: [[f32; 3]; 3]) -> Result<(), String> {
        if corners.iter().flatten().any(|v| !v.is_finite()) {
            return Err("vertex coordinate is not a finite number".into());
        }
        let ids = corners.map(|c| {
            // -0.0 and 0.0 are the same point.
            let key = c.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
            *self.index.entry(key).or_insert_with(|| {
                self.mesh
                    .positions
                    .push(DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64));
                (self.mesh.positions.len() - 1) as u32
            })
        });
        // Triangles that collapse onto an edge or point carry no surface.
        if ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2] {
            self.mesh.triangles.push(ids);
        }
        Ok(())
    }

    fn finish(self) -> TriMesh {
        self.mesh
    }
}

/// Writes binary STL with per-facet normals.
pub fn write_stl_binary(mesh: &TriMesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(84 + mesh.triangle_count() * 50);
    let mut header = [0u8; 80];
    let label = b"DentCraft binary STL";
    header[..label.len()].copy_from_slice(label);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(mesh.triangle_count() as u32).to_le_bytes());
    for i in 0..mesh.triangle_count() {
        let normal = mesh.triangle_cross(i).normalize_or_zero();
        let mut put = |v: DVec3| {
            for c in [v.x, v.y, v.z] {
                out.extend_from_slice(&(c as f32).to_le_bytes());
            }
        };
        put(normal);
        for corner in mesh.triangle(i) {
            put(corner);
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tetrahedron() -> TriMesh {
        TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z],
            vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]],
        )
    }

    #[test]
    fn binary_round_trip_welds_shared_corners() {
        let bytes = write_stl_binary(&tetrahedron());
        assert_eq!(bytes.len(), 84 + 4 * 50);
        let mesh = read_stl(&bytes).unwrap();
        assert_eq!((mesh.vertex_count(), mesh.triangle_count()), (4, 4));
        let original = tetrahedron();
        for i in 0..4 {
            assert_eq!(mesh.triangle(i), original.triangle(i));
        }
    }

    #[test]
    fn ascii_is_parsed_and_degenerate_facets_dropped() {
        let text = "solid t\n\
            facet normal 0 0 1\n outer loop\n vertex 0 0 0\n vertex 1 0 0\n vertex 0 1 0\n endloop\nendfacet\n\
            facet normal 0 0 1\n outer loop\n vertex 0 0 0\n vertex -0 0 0\n vertex 0 1 0\n endloop\nendfacet\n\
            endsolid t\n";
        let mesh = read_stl(text.as_bytes()).unwrap();
        assert_eq!((mesh.vertex_count(), mesh.triangle_count()), (3, 1));
    }

    #[test]
    fn rejects_garbage_and_broken_facets() {
        assert!(read_stl(b"hello").is_err());
        let broken =
            "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nendloop\nendfacet\nendsolid";
        assert!(
            read_stl(broken.as_bytes())
                .unwrap_err()
                .contains("3 vertices")
        );
        let nan =
            "solid t\nouter loop\nvertex nan 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendsolid";
        assert!(read_stl(nan.as_bytes()).is_err());
    }
}
