//! Wavefront OBJ geometry: `v` and `f` statements. Everything else is ignored.

use dentcraft_mesh::TriMesh;
use glam::DVec3;

/// Parses OBJ text; polygons are fan-triangulated and negative indices are relative.
pub fn read_obj(bytes: &[u8]) -> Result<TriMesh, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut mesh = TriMesh::default();
    let mut polygon: Vec<u32> = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => {
                let mut xyz = [0.0f64; 3];
                for value in &mut xyz {
                    *value = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .filter(|v: &f64| v.is_finite())
                        .ok_or_else(|| format!("line {line_no}: bad vertex"))?;
                }
                mesh.positions.push(DVec3::from_array(xyz));
            }
            Some("f") => {
                polygon.clear();
                for word in words {
                    let text = word.split('/').next().unwrap_or("");
                    let index: i64 = text
                        .parse()
                        .map_err(|_| format!("line {line_no}: bad face index '{word}'"))?;
                    let n = mesh.positions.len() as i64;
                    let resolved = match index {
                        0 => return Err(format!("line {line_no}: face index 0")),
                        i if i < 0 => n + i,
                        i => i - 1,
                    };
                    if resolved < 0 || resolved >= n {
                        return Err(format!("line {line_no}: face index {index} out of range"));
                    }
                    polygon.push(resolved as u32);
                }
                if polygon.len() < 3 {
                    return Err(format!("line {line_no}: face with fewer than 3 vertices"));
                }
                for k in 1..polygon.len() - 1 {
                    mesh.triangles
                        .push([polygon[0], polygon[k], polygon[k + 1]]);
                }
            }
            _ => {}
        }
    }
    Ok(mesh)
}

/// Writes OBJ text; coordinates print with `{}`, which round-trips `f64` exactly.
pub fn write_obj(mesh: &TriMesh) -> String {
    let mut out = String::from("# DentCraft\n");
    for p in &mesh.positions {
        out.push_str(&format!("v {} {} {}\n", p.x, p.y, p.z));
    }
    for t in &mesh.triangles {
        out.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_exact() {
        let mesh = TriMesh::new(
            vec![
                DVec3::new(0.1, -2.5e-7, 1.0 / 3.0),
                DVec3::new(1e10, 2.0, -0.0),
                DVec3::new(0.0, 1.0, 123456.789012345),
            ],
            vec![[0, 1, 2], [2, 1, 0]],
        );
        assert_eq!(read_obj(write_obj(&mesh).as_bytes()).unwrap(), mesh);
    }

    #[test]
    fn face_forms_negative_indices_and_quads() {
        let text = "# comment\nmtllib x.mtl\no thing\nv 0 0 0\nv 1 0 0 1.0\nv 1 1 0\nv 0 1 0\n\
            vt 0 0\nvn 0 0 1\ns off\n\
            f 1/1/1 2/1/1 3/1/1 4/1/1\n\
            f 1//1 2//1 3//1\n\
            f 1/1 2/1 -1/1\n\
            f -4 -3 -2\n";
        let mesh = read_obj(text.as_bytes()).unwrap();
        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(
            mesh.triangles,
            vec![[0, 1, 2], [0, 2, 3], [0, 1, 2], [0, 1, 3], [0, 1, 2]]
        );
    }

    #[test]
    fn malformed_input_is_an_error() {
        let head = "v 0 0 0\nv 1 0 0\nv 0 1 0\n";
        for bad in [
            "f 0 1 2",
            "f 1 2 4",
            "f 1 2 -4",
            "f 1 2",
            "f 1 2 x",
            "f 1 2 1.5",
        ] {
            let text = format!("{head}{bad}\n");
            assert!(read_obj(text.as_bytes()).is_err(), "{bad}");
        }
        assert!(read_obj(b"v 0 nan 0\n").is_err());
        assert!(read_obj(b"v 0 inf 0\n").is_err());
        assert!(read_obj(b"v 0 0\n").is_err());
        assert!(read_obj(&[0xff, 0xfe, b'\n']).unwrap().is_empty());
    }
}
