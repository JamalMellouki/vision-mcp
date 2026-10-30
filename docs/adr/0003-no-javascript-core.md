# ADR 0003: No JavaScript engine in the core

## Status

Superseded by [ADR 0004](0004-embedded-javascript-runtime.md).

The original decision is retained below for history.

## Context

The target machines include phones and low-memory ARM devices. Embedding a
JavaScript engine and a DOM implementation would turn Vision into another
browser, with the same startup, memory, and security costs.

## Decision

The renderer does not execute JavaScript. It renders server-rendered HTML and
preloaded resources. Client-rendered applications are captured through
`vision-browser` or `vision-build`, which produce a `Snapshot` and then drop the
browser or build process.

## Consequences

- A static page is rendered without a browser.
- A client-rendered page is not silently treated as static.
- Interactive session actions update the parsed document and layout; they do not
  run page scripts.
- The compatibility report must state what a snapshot lost, so a caller does not
  mistake a partial render for a faithful one.
