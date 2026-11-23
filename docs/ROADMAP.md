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
- A native lifecycle integration with normalized events and explicit wait, immediate redraw, paced redraw and exit controls; preserve Winit's callback-driven event-loop ownership rather than emulating synchronous polling.
- Keep existing adapters until the common API replaces them without breaking integrations.

### Reliability, performance and release hardening
- Fix correctness bugs and edge cases in all supported rendering and input paths.
- Validate dimensions, strides, bounds, transforms, clipping and external buffers before writes.
- Resource lifetime, cache invalidation, bounded scratch/memory use and robust errors.
- Unit, integration, golden-image, property and fuzz tests.
- Cross-platform CI, examples, API documentation and migration guidance.
- Repeatable release benchmarks with before/after comparisons; preserve pixel correctness. Initial Windows/Linux/macOS CI baselines are recorded in [PERFORMANCE.md](PERFORMANCE.md); representative hot spots remain under optimization.
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

## 1.1 V1 execution checklist

This checklist tracks implementation separately from release validation. A checked implementation item does **not** mean the whole V1 exit criteria have passed.

### Implemented foundations

- [x] CPU framebuffer, geometry, transforms and validated pixel storage.
- [x] CPU rasterization for primitives, vector paths, strokes, clipping and antialiasing.
- [x] Linear, radial and conic gradients with serial/parallel paths.
- [x] Texture sampling, transformed drawing, atlas regions, nine-slice and mipmaps.
- [x] Ordered display lists, layers, composition, blending and damage-region APIs.
- [x] TTF/OTF glyph rasterization, OpenType shaping, bidi preparation and bounded glyph caches.
- [x] Retained UI components, layout, focus and platform-independent input state.
- [x] Common host contract and deterministic headless backend.
- [x] Explicit CPU surface export contract (format, stride, alpha and color space).
- [x] Callback-driven Winit/softbuffer host runner with normalized lifecycle/input events and redraw controls.
- [x] Cross-platform CI workflow and CPU benchmark workflow are configured.

### Remaining implementation and verification work

#### P0 — required before calling V1 stable

- [x] Obtain a fully green run of the latest commit across Windows, Linux and macOS, including all feature combinations and the new host-pipeline integration tests (verified on `718157ca3382ea8093c445249b4c30dadaad8ebc`, CI run `45246965509`).
- [ ] Run the native-window examples interactively on Windows, Linux and macOS; record OS version, backend, DPI/scale, resize, minimize/restore, focus, keyboard, pointer, text/IME and clipboard outcomes.
- [ ] Fix every reproducible correctness, lifecycle, rendering or input defect found by those runs.
- [ ] Add deterministic golden/pixel-reference tests for representative rendering paths:
  - [x] Axis-aligned vector path fill with an exact per-pixel reference.
  - [x] Linear gradient stop positions with exact expected pixels.
  - [x] Nearest-neighbor texture scaling and clipping with exact expected pixels.
  - [x] Transformed/bilinear texture sampling with exact expected pixels (`transformed_opaque_bilinear_texture_replaces_destination`).
  - [x] Layer composition, opacity and blend modes with exact expected pixels.
  - [x] Text rendering with a licensed deterministic font fixture and pixel-by-pixel references for clipping, affine rotation/translation, identity equivalence and alpha compositing.
- [ ] Add property-based or fuzz-style tests for malformed geometry, clipping, dimensions, strides, display-list commands and image inputs; keep the regular CI suite deterministic and bounded.
- [ ] Validate the public examples as end-to-end application workflows: vector drawing, images, text, UI interaction, input and presentation.
- [ ] Publish an explicit list of platform limitations and unsupported capabilities based on actual platform runs.

#### P1 — performance and long-running stability

- [ ] Run release benchmarks on reference Windows, Linux and macOS machines and store comparable results with hardware/toolchain/feature metadata.
- [ ] Profile representative mixed scenes, not only isolated raster primitives.
- [ ] Optimize only measured bottlenecks; preserve pixel-equivalence/correctness tests for each optimized path.
- [ ] Stress-test texture, path, glyph and layer caches over repeated insert/evict/invalidate cycles; verify bounded memory and stale-handle behavior.
- [ ] Stress-test repeated resize, minimize/restore, frame presentation and host shutdown.
- [ ] Review allocation failure and oversized-input handling at every public allocation boundary.

#### P2 — release packaging and API readiness

- [ ] Review all public API docs and examples for accurate feature requirements and error behavior.
- [ ] Document migration guidance for the common host/presentation API and transitional adapters.
- [ ] Confirm license notices for bundled font fixtures and all optional dependencies.
- [ ] Prepare a versioned 2D V1 release only after P0 is complete and no critical defects remain.

### Current validation notes

- The common headless host and surface-export paths have unit coverage; the public end-to-end headless presentation/export and minimize/restore paths also have integration tests in `ghostengine/crates/ghostengine/tests/host_pipeline.rs`.
- The latest verified commit `718157ca3382ea8093c445249b4c30dadaad8ebc` passed the complete Windows, Linux and macOS CI matrix, including scalar, SIMD, parallel, native-input, native-window and all-feature configurations (run `45246965509`).
- Automated compilation/tests of native adapters do not prove real desktop behavior. Interactive validation on each target OS remains outstanding until evidence is recorded.
- GPU acceleration, 3D rendering and secure-display support are intentionally outside the 2D V1 scope.

