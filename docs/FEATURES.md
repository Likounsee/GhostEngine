# GhostEngine Cargo features

GhostEngine keeps the CPU renderer available in a headless build. Desktop
presentation, native input and image decoding are opt-in capabilities.

## Current features

| Feature | Enables | Does not enable |
|---|---|---|
| *(default)* | CPU rendering, UI, text and platform-independent input state | Native windowing, clipboard integration, image decoding |
| `window` | The minifb window/presentation adapter | Winit, softbuffer, GPU rendering |
| `native-input` | Winit event translation, keyboard/text/IME input and system clipboard | minifb or window creation |
| `native-window` | Winit window/event-loop integration and softbuffer presentation; implies `native-input` | minifb or GPU rendering |
| `image-decoding` | BMP, JPEG, PNG and PNM decoding into CPU textures | Any window system |

The shared `InputState` and `ComponentTree::handle_window_input` API live
in the platform-independent `input` module. This allows either window adapter
to feed the same retained UI interaction pipeline.

## Common build selections

```sh
# Headless CPU renderer (default)
cargo test -p ghostengine

# Image decoding, still headless
cargo test -p ghostengine --features image-decoding

# minifb window adapter
cargo run -p ghostengine --features window --example browser_frontend

# Winit input + native window presentation, without minifb
cargo run -p ghostengine --features native-window --example native_ui_demo
```

Features are additive Cargo capabilities. Avoid enabling both window adapters
unless an application deliberately needs both: they have separate event-loop
and presentation models.

## Feature design rules

- The CPU renderer remains the mandatory reference implementation.
- Platform adapters must not leak platform types into the core rendering API.
- Native input state and retained UI routing are platform-independent.
- Security invariants belong in the core design and must not be switchable off
  through a Cargo feature. Platform-specific secure-display capabilities will
  be reported as supported, unavailable or unsupported rather than silently
  pretending to protect content.
- New features are added only when their code path is genuinely optional and
  the corresponding build/test matrix can validate it.

## Planned feature groups

These are design targets, **not implemented Cargo features yet**:

- `parallel-rendering`: bounded worker scheduling and deterministic parallel
  rasterization controls.
- `simd`: selectable optimized pixel kernels with a scalar fallback.
- `platform-windows`, `platform-linux`, `platform-macos`: platform-specific
  presentation adapters after each target has a verified implementation.
- `secure-display`: platform-backed protected surfaces, capture restrictions
  where the host OS supports them, and explicit capability reporting.

Do not add a Cargo feature merely to reserve its name. Each planned feature
must have a real implementation, tests, documentation and a clear fallback.
