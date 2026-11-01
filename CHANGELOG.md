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
  convergence beside steep, noisy walls and walls almost parallel to the axis. A margin that lies
  on the scan just past the preparation region's edge is no longer taken as buried.
- **Margin undercut**: an overhang at the margin up to 0.05 mm counts as scan noise; a deeper one
  still stops the design and names its direction.
- **Crowns on real margins**: fixed crowns that failed with "folds or intersects a surface" when
  the margin's first point sat near the crown's reference direction.
- **Plaster (stone) model scans**: the margin search retries with sloped walls and from the top of
  the stump when the click misses it; it never returns a neighbouring tooth's margin. A margin found
  this way says "Check this margin" until you edit or re-trace it.
- **Preparation region**: found on scans with holes, loose sheets, non-manifold edges and fused
  contacts; an implausible region (a loose flap, part of a neighbour) is rejected instead of used.

### Tooth libraries

- **Your own tooth library**: `--tooth-library <folder>` (or `DENTCRAFT_TOOTH_LIBRARY`) designs
  crowns from a folder of STL teeth with a `library.json`; the Anatomy step names the library in
  use. Teeth the library lacks use DentCraft's parametric tooth, and so does a library tooth the
  fit cannot follow on a preparation, with a note saying so.
- **Import a set-up**: `dentcraft-cli library from-setup <stl> <folder>` turns an STL of separate
  library crowns set up in both arches into such a folder, numbering each tooth by its place in
  the arch. Library crowns keep their own surface; spots the crown fit cannot follow are smoothed
  locally (at most 0.3 mm on the test library).

### Interface

- **Viewport-first layout**: the ribbon is gone. A floating toolbar at the top left holds the file
  commands and the Design and Tools menus; view buttons sit at the bottom right.
- **Objects panel**: scans grouped by role, and every tooth's margin, inner surface and crown,
  shown or hidden per group or per tooth, all designed teeth on screen at once. An option hides the
  opposite jaw when the view looks at it from behind.
- **Case setup**: the tooth chart, material and scan roles open in their own screen, for a new
  case and from the toolbar, instead of taking the left side of the window.
- **Floating wizard**: the current step and its tooth chips float over the full-width viewport.
  Parameters scroll while Back, Skip and Next stay at the bottom; the header shows step n of m.
- **Expert navigation**: the toolbar toggles a bottom strip of step icons and Measure. Hover at
  the bottom edge to reveal it, or pin it open. The next unfinished step is highlighted and
  navigation preserves workflow dependencies. Right-click the scene for navigation and framing;
  Expert mode adds the reachable steps and Measure. Right-drag keeps its camera behavior.
- **Object selection in Expert mode**: click a visible scan or design part; Ctrl-click adds or
  removes it without drawing with the active tool. Selected objects have outlines and share Fit,
  Hide and Clear actions in the strip and context menu. Editing a selected tooth still respects
  workflow prerequisites. Background context menus preserve the selection.
- **Apply settings to teeth**: copy the current tooth's inner-surface settings, anatomy shape or
  contact targets to the other teeth in the step, or just the selected teeth. One Undo reverses
  the whole operation. Each tooth keeps its margin, insertion axis, crown position and rotation.
  Copying opens the first changed tooth so Next reviews affected teeth in workflow order.
- `--open expert-strip` starts Expert mode with its strip pinned for repeatable layout checks.

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
