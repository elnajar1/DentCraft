# Contributing rules for people and AI agents

These rules apply to every change, whoever or whatever writes it.

## Clean room

DentCraft is designed independently. A contribution must not contain, or be derived from:

- code, decompiled or disassembled output, symbols or strings from any proprietary dental CAD
  product (exocad, 3Shape, Dentsply Sirona, Medit, Dental Wings or others);
- their tooth, pontic, implant, attachment or articulator libraries, icons, skins, sample cases,
  configuration files or proprietary file formats;
- documentation that is only available under a licence or NDA.

Allowed sources: public dental textbooks and papers, public standards (ISO, FDI), public
documentation of open file formats (STL, PLY, OBJ, 3MF, DICOM), open-source code under a licence
compatible with MIT OR Apache-2.0 (with attribution), and feedback from dental technicians.

Dental workflow vocabulary (margin line, insertion axis, cement gap, connector, minimum thickness)
is common professional knowledge and is fine to use.

## Data

Never commit patient scans or case files. Test fixtures are generated in code or are scans made
for the project and released under the repository licence.

## Engineering

- Geometry runs in `f64` (`glam::DVec3`). Convert to `f32` only for rendering.
- Every geometry algorithm ships with tests, including degenerate input (empty mesh, duplicate
  vertices, zero-area triangles, open boundaries).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` must pass before a commit.
- No `unsafe` (the workspace forbids it).
- Accuracy tests for design geometry (gaps, thickness, clearances) measure true 3D distances, at
  triangle interiors (edge midpoints, centroids) as well as vertices, and include steep walls,
  directions that are not grid-aligned and scan-like noise. A vertices-only check on a flat test
  shape once hid a crown that bound on its walls and cut 11 µm into the die.
- Every background job in the app goes through `crash::spawn_worker` (large stack, panics caught and
  shown as the step's error, crash log written), never a bare `std::thread::spawn`.
- Renderer changes run the offscreen tests once with `WGPU_BACKEND=gl` too: it is the local
  stand-in for CI's Linux llvmpipe, which lacks several downlevel features.

## Crate map

| Crate | Role |
|---|---|
| `dentcraft-mesh` | Triangle mesh, topology, queries and repair |
| `dentcraft-io` | STL, PLY and OBJ import and export |
| `dentcraft-case` | Case model: teeth (FDI), restorations, scans, materials, `.dentcraft` files |
| `dentcraft-workflow` | The design wizard: steps per restoration type and session state |
| `dentcraft-design` | Design algorithms: orientation, insertion axis, inner surface |
| `dentcraft-render` | wgpu mesh renderer used by the app and headless screenshots |
| `dentcraft-cli` | Headless command line |
| `dentcraft` | Desktop application (egui + wgpu) |
