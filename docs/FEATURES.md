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
| `simd` | Safe `wide`-based pixel blending (enabled by default) | Runtime ISA dispatch or GPU rendering |
| `parallel-rendering` | Automatic worker selection for large raster/composition work (enabled by default) | A persistent background worker pool |

The shared `InputState` and `ComponentTree::handle_window_input` API live
in the platform-independent `input` module. This allows either window adapter
to feed the same retained UI interaction pipeline. `InputState` is also re-exported
from `window` when the `window` feature is enabled, preserving the older path.

## Common build selections

```sh
# Headless CPU renderer (default)
cargo test -p ghostengine

# Image decoding, still headless
cargo test -p ghostengine --features image-decoding

# minifb window adapter
cargo run -p ghostengine --features window --example browser_frontend

# Winit input adapter only, without minifb or window creation
cargo test -p ghostengine --no-default-features --features native-input

# Winit input + native window presentation, without minifb
cargo run -p ghostengine --features native-window --example native_ui_demo
```

Both `simd` and `parallel-rendering` are enabled by default. Use
`--no-default-features` to validate the scalar kernels and disable automatic
worker selection. Explicit `*_parallel_with_threads` APIs remain available for
applications that deliberately request a worker count; pass one worker to force
serial execution.

```sh
# Scalar + single-threaded fallback
cargo test -p ghostengine --no-default-features

# SIMD only, without automatic parallel selection
cargo test -p ghostengine --no-default-features --features simd

# Parallel worker selection with scalar blending
cargo test -p ghostengine --no-default-features --features parallel-rendering
```

The GhostEngine CI matrix runs formatting and Clippy, then checks the scalar fallback,
SIMD-only mode, parallel-only mode, native input without minifb, and the complete
all-features build on Windows, Linux and macOS.

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

- `platform-windows`, `platform-linux`, `platform-macos`: platform-specific
  presentation adapters after each target has a verified implementation.
- `secure-display`: platform-backed protected surfaces, capture restrictions
  where the host OS supports them, and explicit capability reporting.

Do not add a Cargo feature merely to reserve its name. Each planned feature
must have a real implementation, tests, documentation and a clear fallback.

## CPU surface export

The core framebuffer remains an owned, tightly packed RGBA buffer. The
platform-independent `surface` API now provides an explicit export boundary
for caller-owned native/display buffers:

- `SurfaceDescriptor` declares dimensions, byte row stride, pixel layout,
  alpha handling and output color space.
- `PixelFormat` supports RGB8, BGR8, RGBA8 and BGRA8.
- `AlphaMode` supports straight, premultiplied and opaque output.
- `ColorSpace` supports sRGB and linear-sRGB output.
- `Framebuffer::write_to_surface` converts into borrowed memory, preserves
  row padding and validates dimensions, stride and destination capacity
  before writing any bytes.

This is a CPU-side interoperability API, not a GPU surface or a platform
window implementation. The framebuffer's internal pixel representation is
unchanged, so existing rasterizers retain their current fast row-major path.


## CPU benchmark protocol

Run the same release benchmark on each comparison machine, with the same
GhostEngine commit, Rust toolchain and feature set:

```sh
cargo run -p ghostengine --release --example cpu_raster_bench -- 50
```

The final argument is the iteration count per workload (the 4K-to-1080p
workload is capped at 20 frames). The benchmark prints the detected CPU,
logical thread count, enabled SIMD/parallel features, Rust version, build
mode, per-operation average time and estimated operation FPS. Use release
builds only; debug results are explicitly marked non-comparable. For fair
results, connect laptops to AC power, select the same OS power mode, close
heavy background tasks, and run the benchmark twice after a warm-up run.
Compare the same named workload rows rather than combining individual FPS
values into a whole-engine frame rate.
