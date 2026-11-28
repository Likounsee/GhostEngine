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
- Keep the public API usable by Archia Browser, ArchiaOS, PeaceEmu and other consumers. Integrating a consumer application (including Amethys Launcher) is a separate follow-up after the engine milestone, not a GhostEngine 2D V1 implementation task.

### 2D V1 exit criteria
2D V1 is complete when the documented 2D feature set is implemented, supported CPU paths pass correctness/bounds/memory-safety tests, headless rendering works without a display server or GPU, the common host API and native adapters pass the Windows/Linux/macOS CI matrix (with platform limitations explicit), examples exercise text/images/vector/UI/input/presentation, performance baselines are recorded, and no known critical correctness or safety defects remain. Manual desktop smoke tests are release-validation evidence, but consumer-launcher integration is deliberately outside this engine milestone.

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

- [x] Obtain a fully green CI matrix for candidate commit `96deb146b2331fc37e66bf0bde1e2403ba1474ad` on Windows, Linux and macOS (CI run `46161589387`): Rust 1.87 passed 6/6 steps; each OS passed 24/24 steps, including format, Clippy, API docs, package contents, all feature configurations, 848 tests, headless examples, the persisted `--locked` Amethys integration, 12 image decodes and all 18 scripted PNG frames. This verifies the automated matrix only; real desktop validation remains outstanding.
- [ ] Run the native-window examples interactively on Windows, Linux and macOS; record OS version, backend, DPI/scale, resize, minimize/restore, focus, keyboard, pointer, text/IME and clipboard outcomes.
- [ ] Fix every reproducible correctness, lifecycle, rendering or input defect found by those runs.
- [ ] Add deterministic golden/pixel-reference tests for representative rendering paths:
  - [x] Axis-aligned vector path fill with an exact per-pixel reference.
  - [x] Linear gradient stop positions with exact expected pixels.
  - [x] Nearest-neighbor texture scaling and clipping with exact expected pixels.
  - [x] Transformed/bilinear texture sampling with exact expected pixels (`transformed_opaque_bilinear_texture_replaces_destination`).
  - [x] Layer composition, opacity and blend modes with exact expected pixels.
  - [x] Text rendering with a licensed deterministic font fixture and pixel-by-pixel references for clipping, affine rotation/translation, identity equivalence and alpha compositing.
- [x] Add deterministic property/fuzz-style regression tests for malformed geometry, clipping, dimensions, strides, display-list commands, texture-cache lifecycles and truncated image inputs; keep the regular CI suite bounded.
- [x] Exercise fallible capacity reservation for display lists and verify allocation/capacity errors leave existing state unchanged.
- [x] Verify framebuffer/texture checked dimensions, surface-buffer overflow, image decoder dimension/allocation limits and texture-cache budget failures.
- [x] Validate an end-to-end headless public-API application workflow: retained UI layout, pointer activation, keyboard activation, text rendering, host presentation and surface export (`public_application_workflow_routes_ui_input_renders_text_and_presents_frame`).
- [ ] Validate the complete public examples as interactive end-to-end workflows on desktop: vector drawing, real image assets, text, UI interaction, input, resize/DPI and presentation.
- [x] Document the distinction between automated application-like prototype coverage and real production application validation in [APPLICATION_INTEGRATION.md](APPLICATION_INTEGRATION.md).
- [ ] Publish an explicit list of platform limitations and unsupported capabilities based on actual platform runs.

#### P1 — performance and long-running stability

- [ ] Run release benchmarks on reference desktop Windows, Linux and macOS machines and store comparable results with hardware/toolchain/feature metadata. The latest CI-hosted three-platform benchmark passed on run `46160071685` for commit `9c56c1a2d7ed3ec2206b478875e7c867f7c4fc0f`; its 4K fractional trilinear results are recorded in [PERFORMANCE.md](PERFORMANCE.md), but virtual runner results are not substitutes for reference desktop measurements.
- [x] Add and execute a repeatable mixed-scene profile (10 page renders and 200 full-scene draw passes at 1600x900) on Windows, Linux and macOS CI; results and limitations are recorded in [PERFORMANCE.md](PERFORMANCE.md).
- [ ] Repeat the mixed-scene profile on reference desktop hardware; virtual-runner results are not physical reference baselines.
- [ ] Optimize only measured bottlenecks; preserve pixel-equivalence/correctness tests for each optimized path.
- [x] Complete and pass a unified long-run cache stress pass covering texture, path, glyph and layer caches over repeated insert/evict/invalidate/resize cycles; verify bounded occupancy, texture accounting and stale-handle behavior (`unified_cache_lifecycle_stays_bounded_under_repeated_churn` passes the Windows/Linux/macOS CI matrix).
- [x] Stress-test repeated headless resize, zero-sized minimize/restore, frame presentation and host shutdown (`repeated_headless_resize_minimize_present_and_shutdown_cycles_remain_consistent`, 1,000 cycles). Native desktop lifecycle stress remains part of the per-OS manual validation gate.
- [ ] Complete a broader audit of infallible convenience APIs and internal rasterization scratch allocations; the core framebuffer, borrowed-texture, surface-export, image-decoding, texture-cache and display-list capacity paths now have explicit checked/fallible coverage.

#### P2 — release packaging and API readiness

- [x] Review the public API docs, feature requirements and documented error behavior; the exact candidate builds the public docs with warnings denied.
- [x] Document migration guidance for the common host/presentation API and transitional adapters (`APPLICATION_INTEGRATION.md` and `FEATURES.md`).
- [x] Confirm bundled font/image fixture licenses and include the font notices in the crate package.
- [ ] Audit licenses and transitive notices for all optional dependencies using an approved dependency-license tool.
- [x] Add a release gate covering CI, desktop validation, reliability, benchmarks, licensing and packaging (`RELEASE_CHECKLIST.md`).
- [x] Add a repeatable per-OS desktop validation record (`PLATFORM_VALIDATION.md`).
- [ ] Prepare a versioned 2D V1 release only after P0 is complete and no critical defects remain.

### Current validation notes

- The release gate and per-OS desktop validation record are documented in `RELEASE_CHECKLIST.md` and `PLATFORM_VALIDATION.md`; they do not count as completed physical desktop tests.
- Final automated candidate: `96deb146b2331fc37e66bf0bde1e2403ba1474ad`, CI run `46161589387` — Windows/Linux/macOS and Rust 1.87 all green.

- The common headless host and surface-export paths have unit coverage; the public end-to-end headless presentation/export and minimize/restore paths also have integration tests in `ghostengine/crates/ghostengine/tests/host_pipeline.rs`.
- Historical robustness commit `ad7f1dfac71bedf259dc45a8a7cf5eb3c9c7e8c7` passed the complete Windows, Linux and macOS CI matrix, including scalar, SIMD, parallel, native-input, native-window and all-feature configurations (run `45249472213`). Later commits require their own green matrix.
- The unified cache lifecycle stress test and randomized damage-region pixel-reference test pass on Windows, Linux and macOS in CI run `45495108223` for commit `3782c8a3149413876db524567f8eb2293426add3`.
- Automated compilation/tests of native adapters do not prove real desktop behavior. Interactive validation on each target OS remains outstanding until evidence is recorded.
- GPU acceleration, 3D rendering and secure-display support are intentionally outside the 2D V1 scope.

