# Documentation

Start here:

- [Getting started](getting-started.md): install, CLI, library, sessions, and
  optional browser capture.
- [Agent integration](agent-integration.md): MCP tools, tool contracts, limits,
  and the recommended observe/act/verify loop.
- [Compatibility](compatibility.md): supported HTML/CSS/JS, known gaps, audit
  policy, and platform matrix.
- [Architecture](architecture.md): crates, render pipeline, and invariants.
- [Roadmap](roadmap.md): implemented work and next milestones.
- [Project card](project-card.md): one-page summary for sharing.
- [Presentation outline](presentation.md): a ten-slide technical presentation.
- [Architecture decisions](adr): accepted decisions and their context.

## Repository map

```text
crates/vision-engine   renderer, DOM, CSS, layout, text, paint, audit
crates/vision-cli      vision command
crates/vision-mcp      JSON-RPC integration server
crates/vision-browser  optional Chromium DevTools adapter
corpus                 local HTML/SVG fixtures and compatibility baseline
fuzz                   out-of-workspace libFuzzer target
docs                   public documentation
```

The renderer is deliberately self-contained. Read
[architecture.md](architecture.md) before adding a dependency or a new pipeline
stage.
