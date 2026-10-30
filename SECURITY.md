# Security Policy

## Supported versions

Only the latest alpha release receives security fixes until `1.0`.

## Reporting a vulnerability

Send a report to `security@asynctric.dev`. Include the affected version, a
minimal reproduction, and the impact you observed. Do not open a public issue
before a fix is available.

You will receive an acknowledgement within seven days. If the report is
accepted, a fix and advisory will be prepared, and credit will be given unless
you ask otherwise.

## Scope

Vision parses HTML, CSS, images, fonts, and trusted local scripts. Parser
panics, excessive memory use, path traversal, and loading resources outside the
document root are security issues.

The engine embeds QuickJS. It is **not a security sandbox**: scripts run
in-process with a small DOM facade, a 64 MB heap limit, a 2 MB stack limit, and a
two-second deadline, but they are trusted code. Do not render untrusted pages
that contain scripts with the native engine. Use `vision-browser` inside an
isolated container for untrusted public pages.

`vision-browser` is a separate adapter. It launches an installed Chromium and
runs page JavaScript there, then copies the resulting DOM and resources into a
snapshot. The DevTools endpoint is bound to localhost, but page subresources are
not limited by the initial URL policy. Capture untrusted pages only inside an
isolated container with a throwaway browser profile. Sandbox bypass flags are
for disposable containers, not for developer workstations.
