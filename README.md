<h1 align="center">DentCraft</h1>

<p align="center">
  <strong>An open-source dental CAD application built in Rust.</strong><br />
  Design crowns, bridges, inlays, onlays and veneers on intraoral or model scans with a guided,
  step-by-step workflow. Clean, fast and precise, on Windows, macOS, Linux and (later) the browser.
</p>

> **Status: early development.** DentCraft is not yet usable for production work and is not a
> certified medical device. See [ROADMAP.md](ROADMAP.md).

## Why

Dental technicians spend their day in CAD software that is closed, Windows-only and priced for
large labs. DentCraft aims to be the open alternative:

- **The workflow technicians already know**, simplified: one guided path per restoration, with
  fewer clicks and clear feedback at every step.
- **Precise and stable**: geometry in double precision, memory-safe Rust, every algorithm tested
  on degenerate input.
- **Runs everywhere**: one codebase for desktop, command line and (planned) browser.
- **Automation built in**: scan segmentation, margin detection and anatomy proposals that run on
  your machine (planned).
- **Multilingual**, including right-to-left languages (planned).

## The design workflow

A case lists the teeth to restore and the restoration on each. DentCraft builds the plan from
that: shared steps, then the steps each restoration needs, tooth by tooth.

| # | Step | Crown | Coping | Inlay / Onlay | Veneer | Pontic |
|---|---|:-:|:-:|:-:|:-:|:-:|
| 1 | Scans | ● | ● | ● | ● | ● |
| 2 | Orientation *(optional)* | ● | ● | ● | ● | ● |
| 3 | Margin line | ● | ● | ● | ● | |
| 4 | Insertion axis | ● | ● | ● | ● | |
| 5 | Inner surface (cement gap) | ● | ● | ● | ● | |
| 6 | Anatomy | ● | | ● | ● | ● |
| 7 | Pontic base | | | | | ● |
| 8 | Occlusion & contacts *(optional)* | ● | | ● | | ● |
| 9 | Sculpt *(optional)* | ● | ● | ● | ● | ● |
| 10 | Connectors *(bridges)* | ● | ● | | | ● |
| 11 | Minimum thickness | ● | ● | ● | ● | ● |
| 12 | Finish & export | ● | ● | ● | ● | ● |

## Build from source

Requires a recent stable [Rust](https://rustup.rs) toolchain.

```sh
cargo run --release -p dentcraft          # desktop app
cargo run --release -p dentcraft-cli -- --help
cargo test --workspace
```

## Code map

| Crate | Role |
|---|---|
| [`dentcraft-mesh`](crates/dentcraft-mesh) | Triangle mesh, topology, queries and repair |
| [`dentcraft-io`](crates/dentcraft-io) | STL, PLY and OBJ import and export |
| [`dentcraft-case`](crates/dentcraft-case) | Case model: teeth (FDI), restorations, scans, materials |
| [`dentcraft-workflow`](crates/dentcraft-workflow) | The design wizard: steps per restoration and session state |
| [`dentcraft-cli`](crates/dentcraft-cli) | Headless command line |
| [`dentcraft`](crates/dentcraft) | Desktop application (egui + wgpu) |

## Contributing

Contributions are welcome. Read [AGENTS.md](AGENTS.md) first: it holds the clean-room rules
(no code, libraries or assets from proprietary dental CAD products) and the checks every change
must pass. Never commit patient scans.

## Licence

MIT OR Apache-2.0, at your option. See [NOTICE](NOTICE).

DentCraft is independent and not affiliated with any dental CAD vendor. It is research software:
designs must be checked by a qualified dental professional before clinical use.

---

<div dir="rtl">

## بالعربي

**DentCraft** برنامج مفتوح المصدر لتصميم التركيبات السنية (CAD)، مكتوب بلغة Rust. بيتبع نفس
خطوات العمل اللي فنيين المعامل متعودين عليها، بس بشكل أنضف وأسهل وأدق. يشتغل على Windows وmacOS
وLinux، ومجاني للجميع.

</div>
