# DentCraft layout brief: exocad-style structure, SolidCraft look

Written in our own words from exocad's public wiki (DentalCAD overview, wizard steps, expert mode,
tooth placement pages). Copy the structure only: no exocad icons, skins, screenshots or text.
Every colour, font, radius and icon comes from DentCraft's SolidCraft-derived theme (`theme.rs`,
`icon_svg.rs`).

## Principle

The 3D viewport fills the window. Controls float over it in a few compact panels, so the scan
is always the largest thing on screen. A technician works through a guided sequence (the wizard)
and can drop into an expert mode for per-tooth control.

## Regions

1. **Top bar** (thin, as now): app name, undo/redo, case name and saved state, view preset menu,
   theme toggle.
2. **Main toolbar**, floating at the top left of the viewport: one row of icon buttons that are
   always available: New, Open, Save, Import scans, Case setup, Wizard/Expert toggle, Tools menu
   (Measure, Section, Screenshot, Export STL), Show distances. Tooltips carry the names. It
   replaces the ribbon, which is removed.
3. **Objects panel** (the "group selector"), floating at the left: every object grouped by type
   (Scans: upper, lower, pre-op, bite, other; Margins; Inner surfaces; Crowns). A checkbox per
   group shows or hides it; an expander lists the objects one by one and, for designed parts,
   per tooth. Right-click a group: transparency for the whole group. Buttons: Show all, and a
   toggle that hides the opposite jaw when the view looks at it from behind. A "Hidden"
   expander lists what is currently hidden. The list grows as the design progresses.
4. **Wizard panel**, floating at the right: the current step only: its title, a one-line
   instruction, its parameters and the tooth chips it applies to, with Back and Next fixed at
   the bottom and "step n of m" at the top. Defaults come from the material and restoration
   type; a change in the wizard applies to every tooth of the step.
5. **Expert strip**, at the bottom edge, in Expert mode: slides up when the pointer reaches the
   bottom edge (a pin keeps it open); icons for every step and tool available now, the next
   wizard step highlighted in the accent colour. Selecting objects in the viewport (Ctrl for
   several) fills the strip with the actions for them. Right-click on the background or an
   object opens a context menu with the same actions; while the wizard runs the menu is short.
6. **View cluster**, bottom right of the viewport: occlusal, facial, lingual, left, right buttons
   (the view arrows).
7. **Case setup screen**, before design: tooth chart with restoration types and bridges,
   material, scans and their roles. It opens for a new case, for a case with no units, and from
   the toolbar. The design screen does not keep the tooth chart on screen all the time.

## Mouse (as exocad users expect)

Right drag rotates about the rotation centre, wheel zooms at the pointer, both buttons drag to
pan (arrow keys too), middle click sets the rotation centre and centres the view.

## Tooth placement step (Anatomy)

A tooth library drop-down with previous/next arrows and a preview list; tabs Simple (move,
rotate, scale with Ctrl = rotate, Shift = scale, Ctrl+Shift = directional scale, axis
constraints, "move all together") and Advanced (morphing of tooth parts, anterior symmetry,
mirroring, keep minimum thickness live, abrasion slider for anteriors/posteriors).
