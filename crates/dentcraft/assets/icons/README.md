# Icons

SVG masters for the application's icons, embedded at build time (`src/icon_svg.rs`). Every icon
is monochrome on a 64 × 64 view box and draws in `currentColor`; the app rasterises it at the
pixel size it is shown and tints it with the theme's ink. Lines are 4–5.6 units wide with round
caps and joins, following SolidCraft's flat set.

Icons from SolidCraft (https://github.com/tangency-labs/solidcraft, Apache-2.0,
`assets/icons/ui/design/svg/`) are copied unchanged or adapted as noted. The rest are drawn for
DentCraft in the same style and released under the repository licence.

| File | Origin |
| --- | --- |
| `eye.svg` | SolidCraft `flat/eye.svg` |
| `eye_off.svg` | SolidCraft `flat/suppress.svg` |
| `close.svg` | SolidCraft `flat/close.svg` |
| `moon.svg` | SolidCraft `flat/moon.svg` |
| `fit.svg` | SolidCraft `flat/fit.svg` |
| `plus.svg` | SolidCraft `flat/plus.svg` |
| `folder.svg` | SolidCraft `flat/folder.svg` |
| `save.svg` | SolidCraft `flat/save.svg` |
| `import_tray.svg` | SolidCraft `flat/import_tray.svg` |
| `export_tray.svg` | SolidCraft `flat/export_tray.svg` |
| `chevron_right.svg` | SolidCraft `flat/chevron_right.svg` |
| `chevron_down.svg` | SolidCraft `flat/chevron.svg` |
| `undo.svg` | SolidCraft `flat/undo.svg` |
| `redo.svg` | SolidCraft `flat/redo.svg` |
| `sun.svg` | SolidCraft `model/render_sun.svg`, made monochrome |
| `check.svg` | SolidCraft `model/finish.svg`, made monochrome |
| `warning.svg` | SolidCraft `model/status_warning.svg`, made monochrome |
| `measure.svg` | SolidCraft `model/measure.svg`, made monochrome, guide line removed |
| `new.svg` | DentCraft, after SolidCraft `model/new.svg` |
| `back.svg`, `next.svg`, `skip.svg` | DentCraft |
| `orient.svg`, `margin.svg`, `axis.svg`, `inner.svg`, `anatomy.svg` | DentCraft (design steps) |
| `view_occlusal.svg`, `view_facial.svg`, `view_lingual.svg`, `view_left.svg`, `view_right.svg`, `view_bottom.svg` | DentCraft (view presets) |
| `ws_crown_bridge.svg`, `ws_model_builder.svg`, `ws_implant.svg`, `ws_splint.svg`, `ws_denture.svg`, `ws_smile.svg` | DentCraft (workspaces) |
