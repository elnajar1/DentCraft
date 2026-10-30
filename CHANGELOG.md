# Changelog

DentCraft is in early development. It is not a cleared or certified medical device: every design
must be checked by a qualified dental professional before clinical use.

## Unreleased

### Real scans

- **Margin line on real scans**: detection works on full arches of adjacent preparations. It
  tolerates ragged scan edges and gaps, never runs onto a neighbouring tooth's ridges, and says
  which stage failed when it cannot find the margin.
- **Smooth margins**: a detected margin is smoothed into a clean curve on the scan. The outline is
  filtered across the tooth; the height of interproximal peaks is kept within about 0.1 mm.
- **Upper and lower jaws in one case**: each tooth now uses its own jaw's occlusal direction.
  Before, teeth on the second jaw got an axis pointing into the root.
- **Inner surface**: a clear message when the insertion axis points into the die, and better
  convergence beside steep, noisy walls.

### Safety

- Background jobs that fail show an error in their step instead of closing the app. Crash logs
  are written to `%APPDATA%\dentcraft\crash-logs`.
- Unsaved work is saved every 30 s. After a crash the app offers to restore it. Each open window
  keeps its own recovery file and never touches another window's.
- Closing the window with unsaved changes asks first: Save, Don't save or Cancel.

## 0.2.0 — 2026-10-29

The first release that designs a single crown from start to finish and exports it as STL.

### Design workflow

- **Orientation**: automatic occlusal plane and midline; every view follows the jaw.
- **Margin line**: one click on the preparation traces the margin along the finish line. Drag,
  insert or delete control points; it holds up to 20 µm scan noise, tall preps and a 20° axis tilt.
- **Insertion axis**: automatic best axis, centred in the undercut-free cone. An undercut map shows
  horizontal depth in millimetres, as a surveyor measures it. The axis tilts with a drag handle.
- **Inner surface**: cement gap and margin seal measured along the surface, undercuts blocked out
  along the axis. A section view shows the gap. Accuracy on test preps: the wall gap is exact and
  the die penetration is below 1 µm.
- **Anatomy**: DentCraft's own parametric tooth library, built from textbook average dimensions
  (no proprietary libraries). The crown is fitted to the margin with the material's minimum
  thickness, and sliders adjust width, depth, height, cusps, wear, rotation and position.
- **Finish & export**: closed, watertight STL per crown, simplified within 2 µm with the margin
  vertices locked.
- Undo and redo for every design edit; design decisions are saved in the case file (format 2).

### Interface

- SolidCraft-style workspace switcher (Crown & Bridge; Model Builder, Implant, Splint, Denture and
  Smile Design planned).
- New SVG icon set in SolidCraft's style; dark theme by default.
- Viewer: colour maps, overlay curves and handles, transparency, section plane, a Measure tool.

### Under the hood

- Mesh toolkit: curvature, smoothing, surface paths, regions, hole filling and error-bounded
  simplification.
- Algorithms for fitting crowns to neighbours and the antagonist, with distance maps. They are in
  the library; the app step follows in the next release.

### Known limitations

- Validated on synthetic preparations; not yet on a range of real clinical scans.
- Single crowns only: bridges, pontics, copings, inlays, onlays and veneers come in 0.4.
- Windows build only in this release; macOS and Linux build from source.

## 0.1.0 — 2026-10-28

Foundations: case model (FDI teeth, restorations, materials), workflow engine, STL/PLY/OBJ
import and export, mesh topology and repair, the desktop app shell with the tooth chart, and the
command line tool.
