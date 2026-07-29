## What changes

<!-- One paragraph. What the change does, from the outside. -->

## Why

<!-- The part that is not in the diff: the problem this solves, or the client
     behaviour that forced it. If a reference emulator settled the question,
     say which and where — a file and a function is enough. -->

## Checklist

- [ ] `cargo fmt --all` — silent
- [ ] `cargo clippy --workspace --all-targets` — silent
- [ ] `cargo test --workspace` — green
- [ ] No client files (`*.mul`, `*.uop`) and no machine-specific paths committed
- [ ] Commit messages carry no tool or model attribution
- [ ] A visible gameplay action ships its sound and animation, not just the state change
