# Roadmap

## 0.1 — Foundations

- [x] Workspace, licences, clean-room rules
- [x] Case model: FDI teeth, restorations, bridges, materials, `.dentcraft` files
- [x] Workflow engine: plan per case, step session with skip / back / re-open
- [x] STL import and export
- [x] PLY and OBJ import and export
- [x] Mesh topology: welding, boundaries, components, repair
- [x] Desktop app: themed shell, 3D viewer, scan import (STL, PLY, OBJ), objects list
- [x] Desktop app: FDI tooth chart, case open and save, guided workflow panel
- [x] SolidCraft-style interface: ribbon, browser panels, light and dark themes
- [x] CLI: mesh info and conversion, case plan

## 0.2 — Preparation tools (in progress)

- [ ] Workspace switcher: Crown & Bridge now; Model Builder, Implant, Splint, Denture and Smile
      Design planned
- [ ] Viewer: colour maps, overlay lines and handles, transparency, section plane, picking
- [ ] Mesh toolkit: curvature, smoothing, shortest paths, region growing, hole filling
- [ ] Orientation: occlusal plane and midline, automatic and adjustable
- [ ] Margin line: curvature-guided tracing, editing and automatic proposal
- [ ] Insertion axis: automatic best axis and undercut map
- [ ] Inner surface: cement gap, margin seal and undercut block-out
- [ ] Undo and redo for every design step

## 0.3 — Single crown, end to end

- [ ] Anatomy from DentCraft's own parametric tooth library
- [ ] Anatomy by mirroring the contralateral tooth or copying the pre-op scan
- [ ] Fit to margin, contacts and antagonist, with minimum thickness
- [ ] Sculpt brushes: add, remove, smooth, flatten, morph
- [ ] Distance maps: contacts, occlusion and thickness
- [ ] Watertight STL export per unit

## 0.4 — Bridges and every restoration type

- [ ] Pontics and pontic base
- [ ] Connectors with cross-section control and area check
- [ ] Copings, inlays, onlays and veneers

## 0.5 — Model Builder workspace

- [ ] Printable models: base, hollowing, drain holes and labels
- [ ] Dies: cut, ditch and pins

## 1.0 — Stable

- [ ] Installers for Windows, macOS and Linux
- [ ] Arabic and right-to-left interface
- [ ] User guide and tutorials
- [ ] Smooth viewing of million-triangle scans

## Later

- Implant abutments, bite splints, dentures and smile design
- Automatic tooth segmentation and anatomy proposals from learned models
- Browser build and an MCP server for AI agents
