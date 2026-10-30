# ADR 0001: Six packages, real code only

## Status

Accepted.

## Context

Vision needs a renderer, a command-line interface, a JSON-RPC integration
server, capture adapters, a build adapter, and a browser build. Putting all of them in one crate
would force every user to compile a browser adapter and every adapter to depend
on the CLI.

## Decision

Use six package names: `vision-engine`, `vision-cli`, `vision-mcp`,
`vision-browser`, `vision-build`, and `vision-wasm`.

The workspace only contains a package when that package has working code and
tests. The remaining package names are reserved in this document and in the
roadmap; they are not empty crates.

The published package name is `vision-engine` because `vision` is taken on
crates.io. The library target is still named `vision`, and the binary is still
named `vision`.

## Consequences

- `cargo install vision-cli` does not pull in Chromium. The engine embeds a
  small QuickJS runtime for trusted local scripts; adapters remain separate.
- A browser adapter can be released and versioned on its own.
- The renderer can be tested without any adapter.
- Users see an honest gap instead of a stub crate.
