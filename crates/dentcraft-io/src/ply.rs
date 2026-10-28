//! PLY, ASCII and binary (little and big endian). Vertex x, y, z and face vertex lists are read;
//! every other property and element is skipped.

use dentcraft_mesh::TriMesh;
use glam::DVec3;

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Ascii,
    Little,
    Big,
}

#[derive(Clone, Copy)]
enum Ty {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Ty {
    fn parse(name: &str) -> Result<Ty, String> {
        Ok(match name {
            "char" | "int8" => Ty::I8,
            "uchar" | "uint8" => Ty::U8,
            "short" | "int16" => Ty::I16,
            "ushort" | "uint16" => Ty::U16,
            "int" | "int32" => Ty::I32,
            "uint" | "uint32" => Ty::U32,
            "float" | "float32" => Ty::F32,
            "double" | "float64" => Ty::F64,
            _ => return Err(format!("unknown property type '{name}'")),
        })
    }

    fn size(self) -> usize {
        match self {
            Ty::I8 | Ty::U8 => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::I32 | Ty::U32 | Ty::F32 => 4,
            Ty::F64 => 8,
        }
    }
}

enum Kind {
    Scalar(Ty),
    List { count: Ty, item: Ty },
}

struct Property {
    name: String,
    kind: Kind,
}

struct Element {
    name: String,
    count: usize,
    props: Vec<Property>,
}

/// Parses a PLY file into a triangle mesh; polygons are fan-triangulated.
pub fn read_ply(bytes: &[u8]) -> Result<TriMesh, String> {
    let marker = b"\nend_header";
    let end = bytes
        .windows(marker.len())
        .position(|w| w == marker)
        .ok_or("missing end_header")?;
    let mut body_start = end + marker.len();
    if bytes[body_start..].starts_with(b"\r\n") {
        body_start += 2;
    } else if bytes[body_start..].starts_with(b"\n") {
        body_start += 1;
    }
    let header = String::from_utf8_lossy(&bytes[..end]);
    let (encoding, elements) = parse_header(&header)?;
    let mut body = Body {
        encoding,
        data: &bytes[body_start..],
        pos: 0,
    };
    let mut mesh = TriMesh::default();
    for element in &elements {
        match element.name.as_str() {
            "vertex" => read_vertices(&mut body, element, &mut mesh)?,
            "face" => read_faces(&mut body, element, &mut mesh)?,
            _ => skip_element(&mut body, element)?,
        }
    }
    let n = mesh.positions.len();
    for (i, t) in mesh.triangles.iter().enumerate() {
        if t.iter().any(|&v| v as usize >= n) {
            return Err(format!("face {i}: vertex index out of range"));
        }
    }
    Ok(mesh)
}

fn parse_header(header: &str) -> Result<(Encoding, Vec<Element>), String> {
    let mut lines = header.lines().map(str::trim);
    if lines.next() != Some("ply") {
        return Err("not a PLY file".into());
    }
    let mut encoding = None;
    let mut elements: Vec<Element> = Vec::new();
    for line in lines {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["format", name, ..] => {
                encoding = Some(match *name {
                    "ascii" => Encoding::Ascii,
                    "binary_little_endian" => Encoding::Little,
                    "binary_big_endian" => Encoding::Big,
                    _ => return Err(format!("unknown PLY format '{name}'")),
                });
            }
            ["element", name, count] => elements.push(Element {
                name: (*name).into(),
                count: count
                    .parse()
                    .map_err(|_| format!("bad element count '{count}'"))?,
                props: Vec::new(),
            }),
            ["property", "list", count, item, name] => {
                let element = elements.last_mut().ok_or("property before element")?;
                element.props.push(Property {
                    name: (*name).into(),
                    kind: Kind::List {
                        count: Ty::parse(count)?,
                        item: Ty::parse(item)?,
                    },
                });
            }
            ["property", ty, name] => {
                let element = elements.last_mut().ok_or("property before element")?;
                element.props.push(Property {
                    name: (*name).into(),
                    kind: Kind::Scalar(Ty::parse(ty)?),
                });
            }
            ["element", ..] => return Err(format!("bad element line '{line}'")),
            ["property", ..] => return Err(format!("bad property line '{line}'")),
            // `comment`, `obj_info` and blank lines.
            _ => {}
        }
    }
    Ok((encoding.ok_or("missing format line")?, elements))
}

struct Body<'a> {
    encoding: Encoding,
    data: &'a [u8],
    pos: usize,
}

impl Body<'_> {
    fn scalar(&mut self, ty: Ty) -> Result<f64, String> {
        if self.encoding == Encoding::Ascii {
            let rest = &self.data[self.pos..];
            let start = rest
                .iter()
                .position(|b| !b.is_ascii_whitespace())
                .ok_or("unexpected end of data")?;
            let len = rest[start..]
                .iter()
                .position(|b| b.is_ascii_whitespace())
                .unwrap_or(rest.len() - start);
            self.pos += start + len;
            let token = std::str::from_utf8(&rest[start..start + len])
                .map_err(|_| "non-text data in ASCII body")?;
            return token.parse().map_err(|_| format!("bad number '{token}'"));
        }
        let size = ty.size();
        let raw = self
            .data
            .get(self.pos..self.pos + size)
            .ok_or("unexpected end of data")?;
        self.pos += size;
        let mut b = [0u8; 8];
        b[..size].copy_from_slice(raw);
        if self.encoding == Encoding::Big {
            b[..size].reverse();
        }
        let b4 = [b[0], b[1], b[2], b[3]];
        Ok(match ty {
            Ty::I8 => b[0] as i8 as f64,
            Ty::U8 => b[0] as f64,
            Ty::I16 => i16::from_le_bytes([b[0], b[1]]) as f64,
            Ty::U16 => u16::from_le_bytes([b[0], b[1]]) as f64,
            Ty::I32 => i32::from_le_bytes(b4) as f64,
            Ty::U32 => u32::from_le_bytes(b4) as f64,
            Ty::F32 => f32::from_le_bytes(b4) as f64,
            Ty::F64 => f64::from_le_bytes(b),
        })
    }

    /// Reads a list length, which must be a non-negative integer.
    fn list_len(&mut self, ty: Ty) -> Result<usize, String> {
        let n = self.scalar(ty)?;
        if n.fract() != 0.0 || !(0.0..=u32::MAX as f64).contains(&n) {
            return Err(format!("bad list length {n}"));
        }
        Ok(n as usize)
    }

    fn skip(&mut self, kind: &Kind) -> Result<(), String> {
        match *kind {
            Kind::Scalar(ty) => self.scalar(ty).map(drop),
            Kind::List { count, item } => {
                let n = self.list_len(count)?;
                if self.encoding == Encoding::Ascii {
                    // Each token needs input bytes, so a huge count fails at the end of data.
                    (0..n).try_for_each(|_| self.scalar(item).map(drop))
                } else {
                    let end = n
                        .checked_mul(item.size())
                        .and_then(|len| self.pos.checked_add(len))
                        .filter(|&end| end <= self.data.len())
                        .ok_or("unexpected end of data")?;
                    self.pos = end;
                    Ok(())
                }
            }
        }
    }
}

/// Elements worth reserving room for: no more than the rest of the body can hold, so a header
/// cannot make a small file allocate a huge buffer.
fn reserve_count(body: &Body, element: &Element) -> usize {
    let min_record: usize = element
        .props
        .iter()
        .map(|p| match (body.encoding, &p.kind) {
            // A digit and a separator.
            (Encoding::Ascii, _) => 2,
            (_, Kind::Scalar(ty)) => ty.size(),
            (_, Kind::List { count, .. }) => count.size(),
        })
        .sum();
    let remaining = body.data.len() - body.pos;
    element
        .count
        .min(remaining / min_record.max(1))
        .min(1 << 20)
}

fn skip_element(body: &mut Body, element: &Element) -> Result<(), String> {
    if element.props.is_empty() {
        return Ok(());
    }
    for _ in 0..element.count {
        for prop in &element.props {
            body.skip(&prop.kind)?;
        }
    }
    Ok(())
}

fn read_vertices(body: &mut Body, element: &Element, mesh: &mut TriMesh) -> Result<(), String> {
    let slot = |axis: &str| {
        element
            .props
            .iter()
            .position(|p| p.name == axis && matches!(p.kind, Kind::Scalar(_)))
            .ok_or_else(|| format!("vertex element has no '{axis}' property"))
    };
    let slots = [slot("x")?, slot("y")?, slot("z")?];
    mesh.positions.reserve(reserve_count(body, element));
    for i in 0..element.count {
        let mut xyz = [0.0; 3];
        for (p, prop) in element.props.iter().enumerate() {
            match prop.kind {
                Kind::Scalar(ty) => {
                    let value = body.scalar(ty)?;
                    if let Some(axis) = slots.iter().position(|&s| s == p) {
                        xyz[axis] = value;
                    }
                }
                _ => body.skip(&prop.kind)?,
            }
        }
        if xyz.iter().any(|v| !v.is_finite()) {
            return Err(format!("vertex {i}: coordinate is not a finite number"));
        }
        mesh.positions.push(DVec3::from_array(xyz));
    }
    Ok(())
}

fn read_faces(body: &mut Body, element: &Element, mesh: &mut TriMesh) -> Result<(), String> {
    if element.props.is_empty() {
        return Ok(());
    }
    mesh.triangles.reserve(reserve_count(body, element));
    let mut polygon: Vec<u32> = Vec::new();
    for i in 0..element.count {
        for prop in &element.props {
            let is_indices = prop.name == "vertex_indices" || prop.name == "vertex_index";
            let Kind::List { count, item } = prop.kind else {
                body.skip(&prop.kind)?;
                continue;
            };
            if !is_indices {
                body.skip(&prop.kind)?;
                continue;
            }
            let n = body.list_len(count)?;
            if n < 3 {
                return Err(format!("face {i}: fewer than 3 vertices"));
            }
            polygon.clear();
            for _ in 0..n {
                let v = body.scalar(item)?;
                if v.fract() != 0.0 || !(0.0..=u32::MAX as f64).contains(&v) {
                    return Err(format!("face {i}: bad vertex index {v}"));
                }
                polygon.push(v as u32);
            }
            for k in 1..n - 1 {
                mesh.triangles
                    .push([polygon[0], polygon[k], polygon[k + 1]]);
            }
        }
    }
    Ok(())
}

/// Writes little-endian binary PLY: float32 positions, faces as `uchar` count + `int` indices.
pub fn write_ply_binary(mesh: &TriMesh) -> Vec<u8> {
    let header = format!(
        "ply\nformat binary_little_endian 1.0\ncomment DentCraft\n\
         element vertex {}\nproperty float x\nproperty float y\nproperty float z\n\
         element face {}\nproperty list uchar int vertex_indices\nend_header\n",
        mesh.vertex_count(),
        mesh.triangle_count()
    );
    let mut out =
        Vec::with_capacity(header.len() + mesh.vertex_count() * 12 + mesh.triangle_count() * 13);
    out.extend_from_slice(header.as_bytes());
    for p in &mesh.positions {
        for c in [p.x, p.y, p.z] {
            out.extend_from_slice(&(c as f32).to_le_bytes());
        }
    }
    for t in &mesh.triangles {
        out.push(3);
        for &v in t {
            out.extend_from_slice(&(v as i32).to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_mesh() -> TriMesh {
        TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y],
            vec![[0, 1, 2], [0, 2, 3]],
        )
    }

    #[test]
    fn binary_round_trip() {
        let mesh = quad_mesh();
        let back = read_ply(&write_ply_binary(&mesh)).unwrap();
        assert_eq!(back, mesh);
    }

    #[test]
    fn ascii_quad_is_fan_triangulated_and_extra_properties_skipped() {
        let text = "ply\r\nformat ascii 1.0\r\ncomment hi\r\nelement vertex 4\r\n\
            property float x\r\nproperty float y\r\nproperty float z\r\nproperty uchar red\r\n\
            property list uchar short tags\r\n\
            element edge 1\r\nproperty int a\r\nproperty int b\r\n\
            element face 1\r\nproperty list uchar int vertex_indices\r\nproperty float q\r\n\
            end_header\r\n\
            0 0 0 255 2 7 8\n1 0 0 255 0\n1 1 0 255 1 9\n0 1 0 255 0\n\
            0 1\n\
            4 0 1 2 3 0.5\n";
        let mesh = read_ply(text.as_bytes()).unwrap();
        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(mesh.triangles, vec![[0, 1, 2], [0, 2, 3]]);
    }

    fn big_endian_triangle() -> Vec<u8> {
        let mut data = b"ply\nformat binary_big_endian 1.0\nelement vertex 3\n\
            property double x\nproperty double y\nproperty double z\nproperty uint8 flag\n\
            element face 1\nproperty list uint16 uint32 vertex_index\nend_header\n"
            .to_vec();
        for p in [[0.0f64, 0.0, 0.0], [1.0, 0.0, 0.5], [0.0, 2.0, 0.0]] {
            for c in p {
                data.extend_from_slice(&c.to_be_bytes());
            }
            data.push(9);
        }
        data.extend_from_slice(&3u16.to_be_bytes());
        for i in [0u32, 1, 2] {
            data.extend_from_slice(&i.to_be_bytes());
        }
        data
    }

    #[test]
    fn big_endian_doubles_and_aliases() {
        let mesh = read_ply(&big_endian_triangle()).unwrap();
        assert_eq!(mesh.positions[1], DVec3::new(1.0, 0.0, 0.5));
        assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
    }

    #[test]
    fn truncated_binary_is_an_error() {
        let bytes = write_ply_binary(&quad_mesh());
        for cut in [bytes.len() - 1, bytes.len() - 20, 10] {
            assert!(read_ply(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let be = big_endian_triangle();
        assert!(read_ply(&be[..be.len() - 3]).is_err());
    }

    #[test]
    fn bad_index_nan_and_missing_header_end() {
        let head = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\n\
            property float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n";
        let verts = "0 0 0\n1 0 0\n0 1 0\n";
        let ok = format!("{head}{verts}3 0 1 2\n");
        assert!(read_ply(ok.as_bytes()).is_ok());
        for faces in ["3 0 1 3\n", "3 0 1 -1\n", "3 0 1.5 2\n", "2 0 1\n"] {
            let text = format!("{head}{verts}{faces}");
            assert!(read_ply(text.as_bytes()).is_err(), "{faces}");
        }
        let nan = format!("{head}0 0 0\n1 nan 0\n0 1 0\n3 0 1 2\n");
        assert!(read_ply(nan.as_bytes()).is_err());
        assert!(read_ply(b"ply\nformat ascii 1.0\nelement vertex 0\n").is_err());
        assert!(read_ply(b"hello").is_err());
    }

    #[test]
    fn huge_counts_with_empty_body_do_not_hang() {
        let with_props = "ply\nformat binary_little_endian 1.0\nelement vertex 4000000000\n\
            property float x\nproperty float y\nproperty float z\nend_header\n";
        assert!(read_ply(with_props.as_bytes()).is_err());
        let unknown = "ply\nformat binary_little_endian 1.0\nelement junk 4000000000\n\
            property int a\nend_header\n";
        assert!(read_ply(unknown.as_bytes()).is_err());
        let ascii_list = "ply\nformat ascii 1.0\nelement junk 1\nproperty list uint uchar a\n\
            end_header\n4000000000 1 2\n";
        assert!(read_ply(ascii_list.as_bytes()).is_err());
        // No properties: skipped without looping over the count.
        let empty = "ply\nformat ascii 1.0\nelement junk 4000000000\nend_header\n";
        let mesh = read_ply(empty.as_bytes()).unwrap();
        assert!(mesh.positions.is_empty());
    }

    #[test]
    fn huge_vertex_count_reserves_no_more_than_the_body_holds() {
        let head = "ply
format binary_little_endian 1.0
element vertex 4000000000
            property uchar x
property uchar y
property uchar z
end_header
";
        let mut bytes = head.as_bytes().to_vec();
        let remaining = 1 << 20;
        bytes.resize(head.len() + remaining, 7);
        assert!(read_ply(&bytes).is_err());
        let (encoding, elements) = parse_header(head).unwrap();
        let body = Body {
            encoding,
            data: &bytes[head.len()..],
            pos: 0,
        };
        let reserve = reserve_count(&body, &elements[0]);
        assert!(reserve > 0 && reserve <= remaining / 3, "{reserve}");
    }

    #[test]
    fn face_element_without_properties_returns_promptly() {
        // Looping over the count would not finish; give up on the reader after five seconds.
        let text = "ply
format ascii 1.0
element face 18446744073709551615
end_header
";
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(read_ply(text.as_bytes())));
        let result = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("read_ply did not return");
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn malformed_element_and_property_lines_are_errors() {
        let ok = "ply
format ascii 1.0
comment c
obj_info o
element vertex 1
            property float x
property float y
property float z
end_header
0 0 0
";
        assert_eq!(read_ply(ok.as_bytes()).unwrap().vertex_count(), 1);
        for (good, bad) in [
            (
                "property float y
",
                "property float
",
            ),
            (
                "element vertex 1
",
                "element vertex 1 x
",
            ),
            (
                "property float z
",
                "property list uchar int
",
            ),
        ] {
            let text = ok.replace(good, bad);
            let err = read_ply(text.as_bytes()).unwrap_err();
            assert!(err.contains(bad.trim()), "{err}");
        }
    }
}
