# Fuzzing

The `html` target feeds arbitrary UTF-8 HTML through the parser, stylesheet
cascade, layout, and paint pipeline. It is intentionally outside the workspace
and is not part of a normal build.

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run html -- -max_len=262144
```

Keep the input cap aligned with `max_html_bytes` so the harness does not spend
its budget on deliberate size-limit rejections.
