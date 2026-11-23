# GhostEngine — Roadmap

A milestone is complete only when its acceptance criteria and tests pass. Scaffolding and documentation alone do not count.

## 1. GhostEngine 2D V1 — current priority

Finish one complete, reliable and reusable 2D engine. There is **no separate 2D V2 milestone**. Prioritize functional completeness, correctness, portability, performance and real application integration over speculative additions.

### Rendering and vector graphics
- Core errors, colors, pixel formats, geometry, transforms and validated render targets.
- CPU rasterization: clear, pixels, rectangles, lines, triangles, circles and rounded rectangles.
- Paths, fill rules, strokes, joins/caps, clipping and antialiasing.
- Linear, radial and conic gradients with consistent alpha compositing.
- Image/texture drawing, filtering, transforms, texture regions and nine-slice rendering.
- Ordered display lists, off-screen surfaces and reusable scratch storage.

### Text, UI and composition
- TTF/OTF loading, glyph rasterization, measurement and baseline layout.
- Shaping, bidirectional text, glyph caching and documented fallback behavior.
- Retained UI, layout, focus, pointer/keyboard/text input and interaction.
- Layers, opacity, blend modes, transforms and correct composition order.
- Damage tracking only when correctness and measurements justify it.

### Public host and presentation API
- One platform-independent application-facing API and deterministic headless host.
- Shared lifecycle, resize, scale-factor, focus, redraw, pointer, keyboard and text/IME events.
- Explicit presentation contract: dimensions, row stride, pixel format, alpha mode and color space.
- Windows, Linux and macOS adapters behind the same API; no OS-specific types in application rendering APIs.
- Typed capability queries and explicit unsupported-feature errors.
- Keep existing adapters until the common API replaces them without breaking integrations.

### Reliability, performance and release hardening
- Fix correctness bugs and edge cases in all supported rendering and input paths.
- Validate dimensions, strides, bounds, transforms, clipping and external buffers before writes.
- Resource lifetime, cache invalidation, bounded scratch/memory use and robust errors.
- Unit, integration, golden-image, property and fuzz tests.
- Cross-platform CI, examples, API documentation and migration guidance.
- Repeatable release benchmarks with before/after comparisons; preserve pixel correctness.
- Validate real application integrations and keep the public API usable by Archia Browser, ArchiaOS, PeaceEmu and other consumers.

### 2D V1 exit criteria
2D V1 is complete only when the documented 2D feature set is implemented, supported CPU paths pass correctness/bounds/memory-safety tests, headless rendering works without a display server or GPU, the common host API is validated on Windows/Linux/macOS (with limitations explicit), examples exercise text/images/vector/UI/input/presentation, CI passes the supported matrix, performance baselines are recorded, and no known critical correctness or safety defects remain.

## 2. Hardware acceleration — directly after 2D V1

This is the next major phase, **not 2D V2**. Keep the CPU renderer fully supported as fallback and correctness reference.

- Define a GhostEngine-owned backend abstraction and resource model.
- Evaluate GPU APIs and platform/driver coverage for Windows, Linux and macOS.
- Implement device discovery, capabilities, resource lifetime, command submission, synchronization, presentation and device-loss handling.
- Implement GPU rendering for the established 2D command/resource model.
- Compare GPU output with CPU reference output and validate on real devices.
- Benchmark end-to-end workloads, including uploads, synchronization and presentation.

No GPU support or performance claim is complete until it works on real hardware and passes correctness tests.

## 3. 3D rendering — after hardware acceleration

Start 3D only after the GPU foundation is working and validated. Reuse the established resource, command and backend architecture.

- 3D vectors, matrices, transforms, meshes and vertex/index buffers.
- Cameras, projections and view transforms.
- Homogeneous clipping, viewport mapping and triangle setup.
- Depth buffer and perspective-correct interpolation.
- Texture sampling, materials, shaders and basic lighting.
- Deterministic test scenes and real-device validation; CPU reference/fallback where practical.

## Integration targets

- **Archia Browser:** compositing surfaces, images, text, transforms and UI; DOM/layout remain outside GhostEngine.
- **ArchiaOS:** framebuffer first, then OS compositor/surfaces; GPU use only when its driver stack exists.
- **PeaceEmu:** pixel surfaces, texture upload and frame presentation.
- **Office suite:** text, vector paths, images, clipping, page rendering and transitions.

Each integration must use the public API, not private renderer internals.

## Execution order

1. Finish and harden **GhostEngine 2D V1**.
2. Implement and validate **hardware/GPU acceleration**.
3. Begin **3D rendering** on top of the GPU/resource architecture.

There is no 2D V2 phase and no separate software-3D phase before GPU acceleration. Do not start 3D while 2D V1 acceptance criteria remain unmet. Never mark a milestone complete based on scaffolding alone.
