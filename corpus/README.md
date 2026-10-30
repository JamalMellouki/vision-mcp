# Corpus

The corpus contains small pages that exercise the renderer without a browser.
They are not golden-image tests yet; the expected output is reviewed as a PNG
and the element rectangles are checked when a feature changes. The compatibility
baseline in `goldens/manifest.json` is enforced by
`crates/vision-engine/tests/corpus.rs`.

`smoke.html` covers block flow, inline text, Arabic shaping, lists, a table with
a caption and colspan, form controls, shrink-to-fit inline-block chips, and an
SVG image on a text baseline. `dot.svg` is the image resource used by the
corpus. `react-ssr.html` is the React 19 server-rendered regression fixture.
