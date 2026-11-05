# GhostEngine

**GhostEngine is an independent, reusable 2D and 3D graphics engine written in Rust.**

Its purpose is to provide a common graphics foundation for:

- **ArchiaOS** — desktop composition, applications and, eventually, direct framebuffer rendering.
- **PeaceEmu** — fast and accurate rendering for emulated systems.
- **Archia Browser** — rendering surfaces and graphics primitives for browser content.
- **The Office suite** — documents, spreadsheets, presentations, text and vector graphics.
- **Third-party applications** — through a stable, documented public API.

GhostEngine is designed to work without requiring a proprietary operating system, a licensed graphics product or a GPU. Its own CPU software renderer is a first-class rendering backend. GPU acceleration is an optional way to improve performance, not a prerequisite for the engine's core functionality.

> **Project status:** early planning stage. The repository is being structured; the capabilities described below are goals, not claims that they are already implemented.

## Design principles

1. **Independent rendering core.** The engine owns its rendering model, resource model, command representation and software rendering implementation.
2. **CPU first, GPU optional.** Essential rendering must work on a CPU-only machine. Hardware backends may accelerate supported workloads.
3. **Portable.** No dependency on Windows, DirectX or a proprietary graphics stack. Platform-specific integration stays behind explicit interfaces.
4. **Reusable.** Applications consume public APIs; they do not need to know which backend executes a frame.
5. **2D and 3D.** A coherent foundation for vector/raster 2D, compositing, and progressively more advanced 3D rendering.
6. **Testable and deterministic where practical.** Rendering algorithms should be testable without a physical GPU or window system.
7. **Careful dependencies.** Prefer well-maintained permissive-licensed dependencies (MIT, Apache-2.0, BSD); review each dependency and its transitive licensing before adoption.

## Target capabilities

### 2D rendering
- Lines, paths, fills, strokes, gradients and clipping.
- Rectangles, rounded rectangles, circles and other primitives.
- Images, textures, transforms, opacity and blend modes.
- Layers, off-screen targets and compositing.
- Text layout, glyph rasterization and font fallback.
- Anti-aliasing and high-DPI coordinate handling.

### 3D rendering
- Meshes, vertex/index buffers and materials.
- Camera and transform systems.
- Triangle clipping, rasterization, depth buffering and perspective-correct interpolation.
- Lighting and shading, introduced incrementally.
- Textures, render targets and a documented shader/material model.
- CPU rendering as a baseline; optional GPU execution for performance.

### Integration and platform support
- Abstract render targets and presentation surfaces.
- Image and font resource loading through explicit interfaces.
- A compositor for combining application surfaces and layers.
- Host-provided windowing and input integration where appropriate.
- Framebuffer output for constrained environments, including a future ArchiaOS bare-metal path.
- Optional GPU backends, isolated from the core API.

GhostEngine is a graphics engine, not a complete window manager or operating-system driver stack. A host application or operating system may provide windows, input, display discovery and hardware drivers. GhostEngine consumes those services through adapters.

## Planned architecture

The intended Rust workspace is split by responsibility:

| Crate | Responsibility |
|---|---|
| `ghostengine-core` | Public types, math, handles, formats, errors and backend-independent contracts |
| `ghostengine-render` | Render commands, passes, resource descriptions and frame execution interfaces |
| `ghostengine-software` | Independent CPU rasterizer and software render targets |
| `ghostengine-2d` | 2D primitives, paths, fills, strokes and transforms |
| `ghostengine-3d` | 3D scene data, geometry, clipping, depth and rasterization |
| `ghostengine-compositor` | Surfaces, layers, blending and composition |
| `ghostengine-text` | Text layout, glyph cache and font integration |
| `ghostengine-image` | Image decoding/encoding adapters and texture upload descriptions |
| `ghostengine-platform` | Framebuffer and host-platform adapters |
| `ghostengine-gpu` | Optional GPU backend contracts and shared GPU resource mapping |
| `ghostengine-backend-vulkan` | Optional Vulkan implementation, only when justified by the roadmap |

This is a target decomposition, not a requirement to create every crate immediately. Modules should begin in the smallest practical workspace and be split when their interfaces and responsibilities are proven.

## Rendering model

Applications submit backend-independent work to GhostEngine. The engine validates and organizes that work, then executes it through a selected backend:

- **Software backend:** the reference implementation and compatibility baseline.
- **GPU backends:** optional acceleration paths. Unsupported features can fall back to software where practical.
- **Framebuffer adapter:** presents software-rendered output to a host-provided framebuffer; it does not replace the operating system's display driver.

The public API should describe what to render, not expose backend-specific handles as the normal application interface.

## Development roadmap

1. **Foundation:** workspace, crate boundaries, errors, math, resource handles, formats and CI.
2. **Software 2D MVP:** pixel buffers, clipping, blending, transforms, primitives, images and deterministic tests.
3. **Text and composition:** fonts, glyph cache, layers, off-screen surfaces and compositor.
4. **Software 3D:** geometry pipeline, clipping, depth buffer, perspective-correct interpolation, textures and basic lighting.
5. **Platform integration:** host-provided surfaces and framebuffer presentation; document the constraints for UEFI/bare-metal use.
6. **GPU acceleration:** backend interface first, then an optional Vulkan backend and parity tests against software output.
7. **Production hardening:** performance, memory budgets, resource lifetime, API stability, examples, documentation and integration tests.
8. **Application integrations:** dedicated examples and adapters for ArchiaOS, PeaceEmu, Archia Browser and the Office suite.

See [docs/ROADMAP.md](docs/ROADMAP.md) for milestones and acceptance criteria, and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the technical design.

## Quality requirements

- Every milestone adds automated tests and a runnable example where possible.
- The software backend must run in CI without a GPU or display server.
- Rendering tests should use small deterministic scenes and pixel comparisons with documented tolerances.
- Backend-specific code must not leak into the public core API.
- Dependencies must have reviewed licenses and documented reasons for inclusion.
- Performance claims must be backed by reproducible benchmarks.

## Building

The Cargo workspace and build instructions will be added with the first implementation milestone. Until then, this repository is in its planning stage.

## License

No project license has been selected yet. Do not assume that the repository is already licensed for redistribution or reuse. A license will be added explicitly before the first public implementation release.
