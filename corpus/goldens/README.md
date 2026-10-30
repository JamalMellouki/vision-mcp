# Golden corpus manifest

`manifest.json` is the checked-in compatibility baseline for the local corpus.
Each entry names a page, its viewport, required tags and ids, and the exact set
of unsupported features the renderer is expected to report.

The integration test in `crates/vision-engine/tests/corpus.rs` renders every
entry and fails when a page gains a new compatibility gap, loses a required
element, or starts loading an incomplete snapshot. Updating the manifest is an
intentional review step; it is not regenerated automatically.

The manifest is deliberately geometry-free so it can run across font builds.
Reviewed PNG baselines will live beside it once the paint output is stable
enough to compare across systems.
