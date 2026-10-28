# DentCraft

Read [AGENTS.md](AGENTS.md): clean-room rules, engineering rules and the crate map apply to every change.

Verification command (must be green before reporting work done):

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

On this Windows machine the toolchain is `stable-x86_64-pc-windows-gnu`; `~/.cargo/bin` may need to be
on `PATH` in Git Bash (`export PATH="$HOME/.cargo/bin:$PATH"`).
