# ADR 0004: Embedded JavaScript runtime

## Status

Accepted. Supersedes [ADR 0003](0003-no-javascript-core.md).

## Context

React 19 and the target framework matrix need client-side rendering and
hydration. Capturing every page through Chromium would make the core dependent
on a browser process, a large binary, and a desktop-class memory budget, which
conflicts with the Termux/ARM target.

## Decision

Embed QuickJS through `rquickjs` in `vision-engine` for trusted local snapshots.
The DOM, event loop, and browser API subset remain our code. Scripts run before
style computation and layout, so DOM mutations are rendered normally.

The runtime is deliberately bounded:

- 64 MB JavaScript heap limit by default, configurable through `Config`.
- 2 MB stack limit by default, configurable through `Config`.
- Two-second script deadline by default, configurable through `Config`.
- Monotonic `setTimeout`/`setInterval` timers, microtasks, and a deterministic
  `Session::advance_time` clock. A config switch disables script execution.
- No `fetch`, `XMLHttpRequest`, WebSocket, or filesystem API.
- External scripts are loaded into `Snapshot` by the snapshot builder; layout
  and paint perform no I/O.
- Missing scripts, unsupported module scripts, script errors, and job failures
  are recorded as unsupported features and block the audit score.

The in-process runtime is not a security sandbox. It is for local, trusted UI
fixtures and prebuilt application snapshots. Rendering arbitrary public pages
with scripts remains out of scope; the browser adapter's isolated container is
the supported path for those pages.

## Consequences

- The renderer can execute React 19 server output, client bundles, and
  hydration bundles without Chromium.
- The binary grows by roughly the QuickJS static library and adapter code.
- DOM and event coverage is intentionally smaller than a browser. Unsupported
  APIs must surface as compatibility errors rather than silent approximations.
- Sessions retain the JavaScript context and dispatch DOM events. Node wrappers
  have stable identity so framework-internal expando properties survive, and
  the prepared document is shared in place rather than cloned per event.
- ADR 0003's no-JavaScript invariant no longer applies.
