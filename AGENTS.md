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

Real cases, the user's tooth libraries and every run output live under `scratch/`, which is
gitignored. Nothing from there goes into a commit, a public screenshot or the README. Tooth
libraries are read from a folder the user chooses (`--tooth-library` or `DENTCRAFT_TOOTH_LIBRARY`),
never bundled; proprietary library formats (such as `.eoff`/`.xml` library files) are not read.

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
| `dentcraft-anatomy` | Parametric tooth library; user tooth libraries (`user_library.rs`) and set-up import (`setup.rs`) |
| `dentcraft-design` | Design algorithms: orientation, margin detection, insertion axis, inner surface, crown build |
| `dentcraft-render` | wgpu mesh renderer used by the app and headless screenshots |
| `dentcraft-cli` | Headless command line |
| `dentcraft` | Desktop application (egui + wgpu) |

## Design direction (standing rules from the project owner)

- **Layout like exocad DentalCAD, look like SolidCraft.** The structure follows exocad's public
  wiki (https://wiki.exocad.com/wiki/index.php/Main_Page): viewport first, floating toolbar, objects
  panel, wizard panel, expert strip, a case setup screen. Colours, radii, fonts and icons come from
  SolidCraft (`theme.rs`, `icon_svg.rs`, `Palette::floating()`). Copy no exocad icons, skins,
  screenshots or text. The brief, in our words: [docs/layout.md](docs/layout.md).
- **Crowns come from the user's tooth library**; the parametric library is the fallback.
- **Integration over precision**: a crown that designs end to end with a clear note beats a
  perfect error. Loosen a hairline tolerance or fall back with a message rather than block.
- **Economy**: small, single-purpose changes; read code by range; don't re-run whole cases when a
  few teeth cover the change.
- The owner writes Egyptian Arabic; answer in Arabic, keep code, commits and docs in English.

## Working on this machine (Windows)

- Toolchain `stable-x86_64-pc-windows-gnu`; in Git Bash run `export PATH="$HOME/.cargo/bin:$PATH"`.
  Disk F: fills up: use one shared `CARGO_TARGET_DIR=F:/Ai/DentCraft-wt/target-shared`.
- Run app binaries through `pwsh -NoProfile -File F:/Ai/DentCraft-wt/tools/noerr.ps1 <exe> <args>`
  (no crash dialogs). Scripted runs need `--screenshot <png>` or the window stays open.
- Real cases: `bash scratch/tools/run-case.sh <case> <exe> [teeth]` (from the repo root, cases in
  `scratch/cases/`, seeds in each `seeds.txt`, output under `$OUT`), and
  `bash scratch/tools/run-real33.sh <exe> <out-dir>` for the canine demo. Set
  `DENTCRAFT_TOOTH_LIBRARY=F:/Ai/DentCraft/scratch/tooth-library-mod` to design from the library.
- Check release binaries with `python -I scripts/check-windows-exe.py <exe>`.

## State and next work (2026-11-01)

Done on main: real-scan margins, inner surface and crowns (full arch 17/24, plaster model 4/4,
bridge case 1/2, canine demo); floating toolbar, view cluster, Objects panel with design parts and
opposite-jaw auto-hide, Case setup modal; the user's tooth library with set-up import and
parametric fallback; floating current-step wizard with fixed navigation, Expert reveal/pin strip
and a scene context menu that respects workflow prerequisites. 0.2.1 is not published: it waits
for the owner's approval.

Next, in order:

1. **Layout slice 3 follow-up**: the floating wizard, Expert strip and scene menu are in place.
   Add viewport object selection (Ctrl for several) and actions scoped to that selection; the
   current menu uses the active workflow tooth. Wizard parameter changes still use the existing
   per-tooth behavior; add an explicit, undoable way to apply them to the step's teeth. Preserve
   fixed-rect click blocking and workflow prerequisites. See docs/layout.md, items 4 and 5.
2. **Tooth 35 with the library hangs** in `build_crown` (`crates/dentcraft-design/src/crown.rs`)
   past 180 s; 45 is its mirror and works. Find the non-terminating loop; add a bound.
3. **Library teeth 13, 16, 33 fall back** to parametric ("folds or intersects"): make the fit
   follow them (`anatomy_step.rs::fit`, `user_library.rs::repair_radial`).
4. **Parametric failures on real cases**: 11/21, 11/24, 03/37 offset edge inversion in
   `offset_margin` (drop inverted offset edges, intersect the neighbours' offset lines, report the
   azimuth); 11/44 placed template not radially single-valued; 11/46 margin not star-shaped about
   the vertex centroid (try the area centroid); 11/26 and 01/16 margin detection.
5. Roadmap 0.3: quality checks before export, then the design report (see ROADMAP.md).

Commit each verified change on a branch with a clear message; merge to main only when the
verification command is green, and leave publishing a release to the owner.
