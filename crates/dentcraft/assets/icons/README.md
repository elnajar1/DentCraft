# Icons

SVG masters for the application's icons, embedded at build time (`src/icon_svg.rs`), all on a
64 × 64 view box. There are two kinds:

- **Monochrome glyphs** for small UI controls draw in `currentColor`; the app rasterises them as
  white masks and tints them with the theme's ink. Lines are 4–5.6 units wide with round caps
  and joins, following SolidCraft's flat set.
- **Coloured command icons** (ribbon commands, view presets, workspaces) follow SolidCraft's
  isometric model icons: passive faces grey (`#e2e8ef` top, `#b8c3cf` left, `#8e9faf` right),
  active faces blue (`#b9ddf8`, `#328bd3`, `#236ead`), outline `#34465b` 2.55 wide with round
  joins, accent arrows `#236ead`. They may use only these colours (and `currentColor` for line
  art on the background); in the dark theme the app swaps each for its dark value from
  SolidCraft's `palette.toml`. Gaps are cut with masks, never painted white.

Icons from SolidCraft (https://github.com/tangency-labs/solidcraft, Apache-2.0,
`assets/icons/ui/design/svg/`) are copied unchanged or adapted as noted. The rest are drawn for
DentCraft in the same style and released under the repository licence.

| File | Origin |
| --- | --- |
| `eye.svg` | SolidCraft `flat/eye.svg` |
| `eye_off.svg` | SolidCraft `flat/suppress.svg` |
| `close.svg` | SolidCraft `flat/close.svg` |
| `moon.svg` | SolidCraft `flat/moon.svg` |
| `plus.svg` | SolidCraft `flat/plus.svg` |
| `chevron_right.svg` | SolidCraft `flat/chevron_right.svg` |
| `chevron_down.svg` | SolidCraft `flat/chevron.svg` |
| `undo.svg` | SolidCraft `flat/undo.svg` |
| `redo.svg` | SolidCraft `flat/redo.svg` |
| `sun.svg` | SolidCraft `model/render_sun.svg`, made monochrome |
| `check.svg` | SolidCraft `model/finish.svg`, made monochrome |
| `warning.svg` | SolidCraft `model/status_warning.svg`, made monochrome |
| `new.svg` | SolidCraft `model/new.svg` (coloured); the white ring round the badge became a mask cut |
| `folder.svg` | SolidCraft `model/open.svg` (coloured); front flap in the pale active blue |
| `import_tray.svg`, `export_tray.svg` | SolidCraft `model/import.svg`, `model/export.svg` (coloured); white knockouts became a mask |
| `measure.svg` | SolidCraft `model/measure.svg` (coloured); the dotted guide drawn solid so it holds at small sizes |
| `fit.svg` | DentCraft: SolidCraft `flat/fit.svg` corner brackets round a blue arch |
| `fit_tooth.svg` | DentCraft: the same brackets round a blue crown |
| `save.svg` | DentCraft (coloured floppy disk) |
| `back.svg`, `next.svg`, `skip.svg` | DentCraft |
| `orient.svg`, `margin.svg`, `axis.svg`, `inner.svg`, `anatomy.svg`, `contacts.svg` | DentCraft (design steps: arch with orientation arrows; die with blue margin ring; die with insertion-axis arrow; die and crown in section with the blue inner layer; blue crown on a grey die; two crowns with a blue contact mark) |
| `jaw_upper.svg`, `jaw_lower.svg` | DentCraft (two arches, the shown jaw in blue) |
| `view_occlusal.svg`, `view_facial.svg`, `view_lingual.svg`, `view_left.svg`, `view_right.svg`, `view_bottom.svg` | DentCraft (arch with a blue arrow for the viewing direction) |
| `ws_crown_bridge.svg`, `ws_model_builder.svg`, `ws_implant.svg`, `ws_splint.svg`, `ws_denture.svg`, `ws_smile.svg` | DentCraft (workspaces) |
