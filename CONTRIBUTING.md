# Contributing to DentCraft

Thank you for helping build an open dental CAD. Dental technicians, dentists, students and
developers are all welcome.

## Ways to help

- **Use it and report what breaks.** Open an issue with the steps, the restoration type and,
  if you can share it, an anonymised scan. Never attach patient data.
- **Describe your workflow.** Tell us how you design a case today and where it costs you time.
  Use the *Workflow request* issue form.
- **Write code.** Pick an issue labelled `good first issue` or `help wanted`, or one from the
  [roadmap](ROADMAP.md), and say in the issue that you are working on it.

## Before you write code

Read [AGENTS.md](AGENTS.md). Two rules matter most:

1. **Clean room.** Nothing may come from proprietary dental CAD products: no code, decompiled
   output, libraries, icons, sample cases or file formats. Design from public literature,
   standards and technician feedback.
2. **No patient data** in the repository, issues or pull requests.

## Development

```sh
cargo run -p dentcraft                    # desktop app
cargo run -p dentcraft-cli -- --help      # command line
```

Every pull request must pass:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Geometry code ships with tests that include degenerate input (empty meshes, duplicate vertices,
zero-area triangles, open boundaries).

## Pull requests

- One change per pull request, with a clear description of what and why.
- Link the issue it closes.
- Screenshots or short recordings for anything visible in the app.
- By contributing you agree that your work is licensed under MIT OR Apache-2.0, like the
  rest of the project.

## Releasing

Before publishing Windows binaries, check that every imported function is wired up:

```sh
python -I scripts/check-windows-exe.py target/release/dentcraft.exe target/release/dentcraft-cli.exe
```

A windows-gnu build linked with LLVM's `dlltool` (or `llvm-ar` renamed to it) drops import
descriptors silently, and the program then crashes with an access violation before it starts. Use
the MinGW `dlltool` that ships with your GNU toolchain.

## Conduct

This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
