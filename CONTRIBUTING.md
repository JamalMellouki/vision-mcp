# Contributing

## Before you start

Open an issue for behaviour changes, new CSS features, and public API changes.
Small fixes and tests can go straight to a pull request.

## Build and test

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo check --manifest-path fuzz/Cargo.toml
```

The browser integration test needs an installed Chromium. Point
`VISION_BROWSER` at the executable to run it; without that variable the browser
test is skipped:

```sh
VISION_BROWSER=/path/to/chromium cargo test -p vision-browser
```

The test corpus lives in `corpus/`. `corpus/goldens/manifest.json` is a
compatibility baseline enforced by an integration test. Reviewed PNG baselines
are still planned; until then, attach the input page and output PNG to any
rendering change.

The out-of-workspace HTML fuzz target lives in `fuzz/`. It is checked in CI but
is not part of the normal workspace build.

## Code style

- Run `cargo fmt` and `cargo clippy` before opening a pull request.
- Public items need rustdoc. Inline comments are reserved for invariants,
  specification references, performance notes, and `SAFETY` blocks.
- Do not add a dependency without a note in the pull request explaining why
  the standard library or an existing dependency is insufficient.
- Do not add files that contain no working code.

## Commit messages

Use Conventional Commits: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`,
`perf:`, `build:`, `ci:`, `chore:`.

## Developer Certificate of Origin

Every commit must be signed off:

```sh
git commit -s -m "feat: add flex layout"
```

The sign-off certifies that you wrote the change or have the right to submit
it under the project license. See https://developercertificate.org/.
