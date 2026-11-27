# GhostEngine 2D V1 — desktop validation record

This record is intentionally separate from CI. A successful compile or headless test does not prove that a native desktop window behaves correctly on a physical machine.

## How to run

From the repository root, use the native UI demo for the Winit/softbuffer host:

```sh
cargo run --manifest-path ghostengine/Cargo.toml --release --features native-window --example native_ui_demo
```

Also run the paced redraw demo:

```sh
cargo run --manifest-path ghostengine/Cargo.toml --release --features native-window --example native_animation_demo
```

For the legacy minifb adapter, if it is being retained for the release:

```sh
cargo run --manifest-path ghostengine/Cargo.toml --release --features window --example browser_frontend
```

Run the two native-window examples on a real interactive desktop session. Do not use a headless CI runner as a substitute. Record the exact commit tested.

## Required record — duplicate once per OS/backend

- Commit:
- Date (UTC):
- OS and version:
- CPU / GPU:
- Desktop session and compositor (Linux):
- Backend: Winit + softbuffer / minifb:
- Display resolution:
- Display scale / DPI:
- Rust toolchain:
- Cargo features:

| Check | Result (PASS / FAIL / NOT TESTED) | Notes / reproduction |
|---|---|---|
| Window opens and first frame is presented | NOT TESTED | |
| Repeated redraw remains responsive | NOT TESTED | |
| Resize smaller/larger; content and drawable size stay synchronized | NOT TESTED | |
| High-DPI / scale-factor change | NOT TESTED | |
| Minimize then restore; rendering resumes | NOT TESTED | |
| Focus loss then regain | NOT TESTED | |
| Pointer movement, press and release | NOT TESTED | |
| Keyboard press/release and focus navigation | NOT TESTED | |
| Text input (including non-ASCII text) | NOT TESTED | |
| IME composition/commit/cancel | NOT TESTED | |
| Clipboard copy/paste or explicit unsupported error | NOT TESTED | |
| Close window; event loop exits cleanly | NOT TESTED | |
| Reopen repeatedly; no stale surface or crash | NOT TESTED | |
| No busy loop while idle/minimized | NOT TESTED | |

## Release rule

Do not change NOT TESTED to PASS based on compilation or CI. For a reproducible failure, record the steps, expected behavior, actual behavior and logs, then add a regression test before closing the item. If a capability is unavailable on a desktop session, record the typed unsupported result and document the platform limitation.
