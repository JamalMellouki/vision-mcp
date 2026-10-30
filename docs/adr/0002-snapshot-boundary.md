# ADR 0002: Snapshot is the render boundary

## Status

Accepted.

## Context

A renderer that fetches stylesheets and images during paint is hard to audit,
hard to reproduce, and easy to make nondeterministic. A renderer that hides all
resource loading in a global loader is also hard to embed.

## Decision

`Snapshot` contains everything a render needs:

- HTML source.
- External stylesheets, keyed by their original `href`.
- Binary resources, keyed by their original `src`.
- Viewport and device pixel ratio.
- Initial scroll, focus, and form values.

The caller is responsible for reading files or capturing a client-rendered page.
The engine has no `Loader` trait and no filesystem access in its render path.

## Consequences

- A saved snapshot is reproducible and serializable.
- The CLI can preload linked stylesheets and `<img>` resources from the local
  document directory.
- A browser or build adapter produces the same type the static path produces.
- CSS `url()` resources and `@import` are not loaded automatically yet; they are
  explicit future work.
