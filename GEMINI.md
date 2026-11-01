# DentCraft

Read [AGENTS.md](AGENTS.md) first: clean-room rules, data rules, design direction, how to run
things on this machine, and the next work in order.

Verification (must be green before reporting work done):

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```
