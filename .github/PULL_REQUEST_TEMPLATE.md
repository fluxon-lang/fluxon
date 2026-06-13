<!-- Keep the PR title short: what changed. -->

## What changed

<!-- One or two sentences: what this PR does and why. -->

## Type

- [ ] New battery / feature
- [ ] Bug fix
- [ ] Performance
- [ ] Documentation
- [ ] Refactor (no behavior change)

## Checklist

> Commands run inside `runtime/`.

- [ ] `cargo build --locked` — compiles
- [ ] `cargo test --locked` — all tests green
- [ ] `cargo fmt --check` — formatted
- [ ] `cargo clippy --all-targets -- -D warnings` — 0 warnings
- [ ] `cargo run -- run examples/demo.fx` — smoke test works
- [ ] **Test added** for the new behavior
- [ ] The change is one logical unit (battery + refactor not mixed)

## Additional

<!-- Related issue (#NN), warning about a broken invariant, reasoning behind a decision. -->
