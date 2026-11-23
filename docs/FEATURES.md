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

The framebuffer stores tightly packed straight-alpha RGBA pixels. Native RGB-only
presentation adapters convert each pixel by compositing it over opaque black before
packing `0x00RRGGBB`; transparent pixels therefore do not leak their hidden RGB channels.

The high-level `DisplayList::draw_text` and `draw_text_box` methods prepare OpenType shaping,
paragraph-level bidirectional ordering and positioned glyph runs while recording. Replaying a
display list therefore does not reshape text every frame. Applications that need a multi-face
fallback chain can use `draw_shaped_text_box` or `draw_bidi_shaped_text_box`; the lower-level
framebuffer drawing functions remain available for simple glyph-by-glyph rendering.

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
# Decode an image, compare nearest/bilinear sampling and render a clipped mipmapped copy
cargo run -p ghostengine --features image-decoding --example image_pipeline_demo -- image.png output.ppm

# Display-free application using HostBackend
cargo run -p ghostengine --example headless_host_demo

# Headless vector paths, Bézier curves, transforms, strokes and clipping
cargo run -p ghostengine --release --example vector_path_showcase -- output.ppm

# minifb window adapter
cargo run -p ghostengine --features window --example browser_frontend

# Winit input adapter only, without minifb or window creation
cargo test -p ghostengine --no-default-features --features native-input

# Winit input + native window presentation, without minifb
cargo run -p ghostengine --features native-window --example native_ui_demo
cargo run -p ghostengine --features native-window --example native_animation_demo
```

The native UI example uses `HostRunControl::Wait` for an event-driven interface that sleeps
between input/redraw events. The native animation example uses
`HostRunControl::RedrawAfter(Duration)` to request paced frames without a busy loop or direct
Winit dependency. Use `Redraw` for intentionally continuous rendering and `Exit` to finish
the native event loop. A zero `RedrawAfter` duration is unthrottled; use a non-zero interval
for pacing.

Paced and continuous redraw requests are suspended while the normalized drawable size is
zero. Pending animation deadlines are discarded while minimized, so they cannot wake the
window in a redraw loop; restoring/resizing requests a fresh frame and resumes the application's
chosen frame schedule. `HostWindowConfig::with_resizable(false)` can be used to request a fixed-size
native window without depending on platform-specific window attributes.

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
SIMD-only mode, parallel-only mode, native input without minifb, the native window
backend without default features, and the complete all-features build on Windows,
Linux and macOS.

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

## Cross-platform host API direction

The intended application-facing API is shared across Windows, Linux and macOS.
Platform-specific implementations are selected behind that API; applications
must not need three separate window, input or presentation code paths.

The current `window` (minifb) and `native-window` (winit + softbuffer)
features are transitional adapters, not the final public platform architecture.
The CPU rendering core, `InputState`, `SurfaceDescriptor` and framebuffer
export are platform-independent building blocks.

The platform-neutral contract is implemented in `host.rs` and re-exported
from the crate root. It defines validated window configuration, backend
creation/closure, normalized lifecycle/resize/focus/redraw/input events,
physical drawable sizing, scale-factor reporting, capability discovery and
framebuffer presentation without exposing native handles.

The deterministic `HeadlessHost` backend is implemented in `headless.rs`. It
supports ordered event delivery, resize notifications, lifecycle closure,
framebuffer presentation and explicit rejection of operations after closure.
Its tests cover these behaviors without a display server. The existing Winit/softbuffer integration now exposes
`NativeWindowSurface::run_with_host_events`, which translates native resize,
scale-factor, focus, close, redraw and accumulated input into common `HostEvent`
values. The native UI demo consumes those events rather than Winit types. The
native adapter also exposes runtime `HostCapabilities` reporting, including
whether the system clipboard was actually initialized. `NativeClipboard::reconnect`
allows an application to retry clipboard initialization if the desktop service becomes
available later; the native UI example exposes this recovery action. Its control-based runner
supports `HostRunControl::Wait`, `HostRunControl::Redraw`, paced `HostRunControl::RedrawAfter(Duration)` and `HostRunControl::Exit` outcomes, so applications can animate without
importing Winit types or emulating synchronous polling. The native adapter is
still not an implementation of the poll-based `HostBackend` trait, and real
window behavior must still be validated on all three operating systems.
The native adapter deliberately remains callback-driven: Winit owns event dispatch
and redraw scheduling, while `HostBackend::poll_events` is a synchronous polling
contract. These lifecycle models must not be joined by blocking, synthetic
polling, or an adapter that hides event-loop ownership. The public host API now exposes both lifecycle models explicitly: synchronous
polling through `HostBackend` for headless/poll-driven hosts and callback-driven
frame scheduling through `HostRunControl` for native event loops. Remaining host
work is real-window validation on Windows, Linux and macOS and any platform-specific
capability corrections revealed by that validation.

## Planned feature groups

These are design targets, **not implemented Cargo features yet**:

- `platform-windows`, `platform-linux`, `platform-macos`: platform-specific
  presentation adapters after each target has a verified implementation.
- `secure-display`: platform-backed protected surfaces, capture restrictions
  where the host OS supports them, and explicit capability reporting.

Do not add a Cargo feature merely to reserve its name. Each planned feature
must have a real implementation, tests, documentation and a clear fallback.

## 2D gradient rasterization

The CPU rasterizer supports multi-stop linear, radial and conic gradients.
Conic gradients use a clockwise angular coordinate in framebuffer space and
share the same alpha-compositing rules as the other gradient fills. The
transformed-rectangle API also supports conic gradients with affine transforms
and an explicit clip rectangle through
`fill_transformed_rect_conic_gradient_clipped`.

Serial and row-parallel conic gradient fills are covered by equivalence tests.
The transformed path rejects singular/non-finite transforms and avoids writes
outside the supplied clip or framebuffer bounds. The ordered display-list API
also records and replays linear, radial and conic gradients while preserving
the active affine transform and nested clip state.

## Nine-slice texture rendering

The CPU texture API supports clipped nine-slice scaling for resizable panels and
UI frames. The four border margins are preserved where possible and proportionally
compressed when the destination is smaller than the combined borders.
`draw_texture_nine_slice_clipped_with_scratch` reuses the bilinear sampling
workspace across all nine patches and across frames; applications rendering
repeated UI panels should retain one `TextureDrawScratch` per render context.

## Display-list atlas-region commands

The ordered `DisplayList` API can record a draw from a source rectangle inside a
shared texture, making atlas sprites available to retained command lists. Replay
preserves submission order and captured clipping, and uses the renderer's reusable
texture-sampling scratch buffer. Region commands currently require an identity
affine transform; invalid source bounds and empty rectangles are rejected while
recording. The direct texture-region API remains available for callers needing
other rendering arrangements.

## Display-list nine-slice commands

The ordered `DisplayList` API can record nine-slice texture commands using shared
`Arc<Texture>` storage. Replay preserves command order and the clip captured at
record time, and reuses the renderer's `TextureDrawScratch` across all nine patches
and successive frames. Nine-slice commands are currently framebuffer-axis-aligned;
an active affine transform is rejected when recording instead of silently applying
incorrect border scaling. Use the direct texture API when transformed nine-slice
rendering is required.

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
