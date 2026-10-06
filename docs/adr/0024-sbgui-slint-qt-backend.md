# Render the desktop client with Slint on the Qt backend, on Windows

`crates/sbgui` is the desktop client, and its UI is currently built on GPUI,
pinned to a pre-1.0 Zed revision. The defect that motivated this decision is
text: Chinese glyph fallback, and input-method composition, are rendered by
GPUI itself, and have been observed to misbehave and to crash during
composition. The engine underneath is sound — `crates/client-core` owns the
core, the clash_api channel, subscriptions, the OS proxy and the settings, and
publishes a `ClientSnapshot` for a UI to draw and accepts a `ClientCommand` for
a UI to send — so only the renderer is in question.

The decision is to render the desktop client with Slint, using Slint's Qt
backend, on Windows only. Qt carries the window's input-method protocol through
its native integration (`inputMethodEvent` / `inputMethodQuery`), so composition
is the platform's, not a reimplementation; Slint supplies the declarative,
retained-mode widget tree, and the UI stays in Rust, calling `ClientController`
directly with no FFI. This keeps the low-memory requirement reachable — a
retained-mode toolkit draws only what changed, so none of GPUI's GPU context,
driver and texture atlas need to exist — while the language of the composition
candidate window is delegated to the one implementation Microsoft tests.

Windows-only is deliberate: it is the only platform this client ships on, and it
removes the Linux X11/Wayland rendering and IME variants from the boundary.
Slint's own software and winit/Skia backends were considered and rejected for
the same reason GPUI is being replaced — their composition is self-drawn, and
Slint's historical CJK input defects (#1644 Chinese, #1706 Korean, #5982
Chinese composition) were winit-side.

## The boundary

- The engine seam does not move. Slint reads `ClientController::snapshot()` and
  sends `ClientCommand`, exactly as GPUI did; the 400 ms change-detecting poll is
  kept, because redrawing an unchanged window is what the current code already
  avoids. Whether the engine later moves into a separate process is a separate
  decision; this ADR fixes only the renderer, at the existing seam.
- Every screen and overlay keeps the `SBGUI_*` environment-variable scene hooks
  that `scripts/sbgui-shot/` drives, so the headless screenshot harness — the
  existing, highest verification seam — is reused rather than replaced.
- Pure view logic (parse, formatting, locale copy, state transitions) is
  separated from Slint so it can be unit-tested on the host, where the project
  permits only pure-function tests and compilation.
- Window chrome that is already Win32 stays Win32: the custom titlebar is drawn
  in Slint, but the DWM rounded corners and the `WM_ENDSESSION` subclass that
  restores the OS proxy on logoff remain direct `windows-rs` calls, independent
  of the UI toolkit. The single-instance lock is unchanged.
- Assets stay inside the crate: the brand icon and the bundled-font decision are
  the crate's, and the Qt backend reads the OS font stack, so no CJK font is
  shipped.

## Not chosen

Tauri and any WebView front end: a browser engine per window is the opposite of
the low-memory target.

`egui` and `iced`: both are self-drawn and GPU-first, in GPUI's ~100 MB class,
and their CJK input-method handling is the same winit-side problem this ADR
exists to leave.

Qt Widgets in C++: it would render and handle IME well, but it would either
duplicate `client-core` or add an FFI/IPC boundary that Slint does not need,
because a Slint UI is the same Rust process.

Native Win32 controls: the lowest memory and the best IME, but the largest
amount of UI code for nine pages and their overlays.

Supporting Linux as well: out of scope by decision, not by omission.

## Consequences

Qt becomes a build-time and runtime dependency, distributed as DLLs under
LGPLv3 (dynamic linking); Slint is GPLv3 / royalty-free / commercial, and the
project has accepted those terms. Both are recorded here because they constrain
packaging. The LGPLv3 obligations that follow from dynamically linking Qt
Core/Gui/Widgets are: ship the unmodified Qt DLLs and the LGPLv3 license text
plus a prominent notice, offer the corresponding Qt source, keep the DLLs
replaceable and re-linkable (no signing or locking that forbids replacement),
and impose no further restriction on those freedoms. `scripts/package-sbgui-slint.ps1`
writes the notice into the release zip, and `packaging/windows/README.md` is the
packaging-side record. Slint is pinned to an exact version because its API is
not stable: 1.18.1, with Qt 6.10.3 (MSVC kit).

Slint 1.18.1's build-time compiler enables `annotate-snippets`, which requires
`unicode-width ^0.2.2`, while `sbtui`'s ratatui 0.29 pins `unicode-width` to
`=0.2.0`; the two semver-compatible requirements cannot coexist. The accepted
resolution is a vendored `annotate-snippets` with that one requirement relaxed
to `=0.2.0`, wired through a workspace `[patch.crates-io]` recorded at the patch
site in the root `Cargo.toml`.

During the migration the crate builds two binaries — the existing GPUI `sbgui`
and the new `sbgui-slint` — over one framework-agnostic library; the GPUI
binary is removed and `sbgui-slint` renamed to `sbgui` only in the contract step,
after the real-Windows acceptance passes.

The pending IME work in `.scratch/gui-completion` and
`.scratch/sbgui-progressive-workspace` is subsumed: the acceptance they describe
(Chinese composition, a candidate window that follows the caret) is now part of
this port's real-Windows verification, and the screenshot harness remains the
regression gate. A memory measurement on real Windows — the whole point — is not
yet on record and must accompany the port.
