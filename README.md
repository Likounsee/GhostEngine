<p align="center">
  <img src="docs/assets/ghostengine-logo.png" alt="GhostEngine logo" width="420">
</p>

# GhostEngine

**GhostEngine is an independent, lightweight 2D and 3D graphics engine written in Rust.**

Its goal is to implement its own rendering technology: geometry processing, rasterization, compositing, resource management and rendering algorithms. GhostEngine is not a wrapper around an existing graphics engine or graphics API.

The first targets are **Windows and Linux**. ArchiaOS is a possible future platform, but it is not an immediate development target. The first interactive application target is a browser-style frontend rendered entirely by GhostEngine's CPU framebuffer.

> **Project status:** the reusable CPU 2D core includes clipped primitives, anti-aliased drawing and fills, paths and strokes, gradients, affine transforms, nested clipping, texture sampling, transformed layer composition with source-over, multiply, screen and add blending, display lists, retained UI components, TrueType/OpenType glyph rasterization, text layout and shaping. The optional `image-decoding` feature decodes BMP, JPEG, PNG and PNM files into CPU textures. Optional window adapters present completed CPU framebuffers and forward input. The software 3D pipeline is not implemented yet. The 2D implementation is covered by deterministic unit tests and headless examples; performance work is measured with `cpu_raster_bench`.

## Vision

Build a graphics engine that is:

- **Fully self-developed:** rendering algorithms and execution pipeline belong to GhostEngine.
- **Lightweight:** small core, modular features, controlled memory use and minimal mandatory dependencies.
- **Fast:** multithreaded CPU rendering, SIMD optimizations, efficient data layouts and minimal memory traffic.
- **Portable:** Windows and Linux first, with platform-specific presentation isolated behind small adapters.
- **Reusable:** one engine for PeaceEmu, Archia Browser, the Office suite, ArchiaOS in the future, and third-party applications.
- **Testable:** deterministic software rendering and automated image-based tests without requiring a GPU.

## No existing graphics API

GhostEngine will **not use Vulkan, DirectX, OpenGL, WebGPU, or an existing graphics engine** as its rendering foundation.

We will implement our own:

- 2D and 3D rendering pipelines.
- Rasterizers and pixel processing.
- Geometry transformations, clipping and interpolation.
- Depth testing, blending and compositing.
- Resource and memory management.
- Render command representation and execution.
- Optimization and scheduling systems.

The operating system still has to display the final image. On Windows and Linux, a thin platform adapter will use the native system facilities needed to create or present a window/surface and transfer the finished image. Those facilities are presentation plumbing, not rendering engines: the image itself is produced by GhostEngine.

GPU acceleration is **not required** for the initial engine. Any future GPU work must follow a separately designed GhostEngine-owned architecture; it must not make an external graphics API a mandatory dependency. Hardware access and driver compatibility are difficult platform-specific problems and will be approached only after the software renderer is mature.

## Performance and efficiency

Performance is a core requirement, not a later polish step.

### CPU rendering

The first renderer will be a first-class software renderer, designed to use modern CPUs efficiently:

- Multithreaded rendering with independent work units.
- Tile-based rasterization to improve parallelism and cache locality.
- SIMD paths where supported (for example SSE/AVX on x86 and NEON on ARM).
- Efficient pixel formats and memory layouts.
- Reuse of buffers and temporary storage.
- Minimal allocations and avoidable memory copies.
- Early rejection of invisible or unnecessary work.
- Benchmarks for throughput, latency and memory consumption.

Optimizations will be selected from measurements and profiling. Architecture-specific optimizations must retain a correct portable fallback.

### Lightweight by design

- A small core with optional modules.
- No mandatory GPU runtime or graphics API.
- No unnecessary background services.
- Explicit resource lifetimes and predictable memory ownership.
- Dependencies added only when they provide clear value.
- Applications should be able to include only the capabilities they need.

The engine will not promise performance figures before reproducible benchmarks exist.

## Planned capabilities

### 2D

- Pixels, lines, rectangles, rounded rectangles, circles and paths.
- Fills, strokes, gradients, clipping and transforms.
- Images, textures, opacity and blend modes.
- Anti-aliasing and high-DPI coordinate handling.
- Layers, off-screen targets and compositing.
- Text layout, glyph rasterization and font integration.

### 3D

- Vertices, indices, meshes and materials.
- Model, view and projection transformations.
- Triangle clipping and rasterization.
- Perspective-correct interpolation.
- Depth buffering and visibility handling.
- Textures, lighting and shading.
- Render targets and a documented material/shader model.

The 3D pipeline will be implemented incrementally, with a software reference path and tests for each stage.

## Target architecture

The intended architecture separates the rendering core from platform integration:

```text
Applications
  |  PeaceEmu / Archia Browser / Office / other software
  v
GhostEngine public API
  |
  +-- 2D and 3D rendering
  +-- Render commands and resource management
  +-- CPU software rasterizer
  +-- Compositor
  |
  v
GhostEngine platform adapter
  |
  +-- Windows presentation
  +-- Linux presentation
  |
  v
Native window / display facilities
```

The platform adapter is deliberately small. It handles the operating-system boundary; it does not decide how primitives, triangles, text or effects are rendered.

### Proposed Rust modules

The final crate boundaries will be chosen as implementation solidifies. The initial workspace should stay compact rather than creating many empty crates prematurely.

| Module | Responsibility |
|---|---|
| `core` | Public types, math, errors, formats and backend-independent contracts |
| `render` | Render commands, passes, frame execution and resource descriptions |
| `software` | CPU rasterizer, pixel buffers and software render targets |
| `2d` | 2D primitives, paths, fills, strokes and transforms |
| `3d` | Geometry pipeline, clipping, depth and 3D rasterization |
| `compositor` | Layers, surfaces, blending and composition |
| `text` | Text layout, glyph cache and font integration |
| `image` | Image resources and decoding/encoding adapters |
| `platform` | Minimal Windows and Linux presentation adapters |
| `gpu` | Reserved for a possible future, independently designed hardware path |

## Development priorities

1. **Project foundation:** Rust workspace, formatting, linting, CI, errors, math and basic public types.
2. **CPU renderer foundation:** pixel buffers, formats, clipping, blending, deterministic output and tests.
3. **Fast 2D renderer:** primitives, transforms, paths, tile-based execution and multithreading.
4. **Images and composition:** image resources, layers, off-screen surfaces and compositor.
5. **Text:** glyph rasterization, layout and caching.
6. **Software 3D:** transformations, clipping, triangle rasterization, depth, textures and basic lighting.
7. **Windows and Linux presentation:** minimal native adapters to display GhostEngine-produced frames.
8. **Optimization and hardening:** profiling, SIMD, memory budgets, benchmarks, stress tests and API stability.
9. **Application integration:** PeaceEmu, Archia Browser and the Office suite.
10. **Future hardware research:** investigate whether a GhostEngine-owned GPU/driver architecture is viable without relying on existing graphics APIs.
11. **Future ArchiaOS port:** only when ArchiaOS is ready to integrate an independent graphics engine.

See [docs/ROADMAP.md](docs/ROADMAP.md) for milestones and acceptance criteria, and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the technical design.

## Quality requirements

- The CPU renderer must work without a GPU.
- Core rendering tests must run in CI without a display server.
- Small deterministic scenes should have pixel-output tests.
- Multithreaded and single-threaded output should be consistent.
- Portable fallback implementations must exist for optional SIMD paths.
- Benchmarks must measure rendering time and memory use on documented hardware.
- Performance changes must be validated against representative workloads.
- Platform-specific code must not leak into the rendering core.
- Dependencies and their licenses must be reviewed before adoption.

## Building

The initial workspace lives in the `ghostengine/` directory. From the repository root:

```bash
cd ghostengine
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test -p ghostengine --features image-decoding
cargo run -p ghostengine --example first_image -- first-image.ppm
cargo run -p ghostengine --features window --example browser_frontend
cargo run -p ghostengine --features native-window --example native_ui_demo
cargo run -p ghostengine --release --example cpu_raster_bench -- 5
```

The `first_image` example creates a 320×180 PPM image headlessly. The `browser_frontend` example (enable `window`) opens a native desktop window, draws a browser-like interface with GhostEngine's CPU renderer, and accepts mouse/keyboard input. The optional `minifb` dependency is used only for window creation, input and presenting GhostEngine's completed pixel buffer; it does not render the scene. The current CI checks formatting and Clippy, then runs the unit tests.

## License

No project license has been selected yet. Do not assume that the repository is already licensed for redistribution or reuse. A license will be added explicitly before the first public implementation release.
