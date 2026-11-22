# GhostEngine — Roadmap

This roadmap is ordered by dependencies. A milestone is complete only when its acceptance criteria and tests pass; a folder or placeholder API does not count as implementation.

## 0. Repository foundation

**Deliverables**
- Rust workspace and documented minimum supported Rust version.
- Formatting, linting and test commands.
- CI for formatting, linting, unit tests and documentation.
- License decision and dependency policy.
- Architecture decision records for major API choices.

**Acceptance criteria**
- Clean checkout builds and tests in CI.
- No mandatory GPU, OS windowing or proprietary SDK dependency.
- CI results are visible and reproducible.

## 1. Core types and software 2D MVP

**Deliverables**
- Core errors, pixel formats, colors, integer/float geometry and transforms.
- Safe render-target and pixel-buffer descriptors.
- Software backend with clear, pixel write, clipping and blending.
- Solid rectangles, lines and triangles.
- Small example that writes a rendered image to a file or memory buffer.

**Acceptance criteria**
- Tests cover bounds, stride, zero-sized/invalid targets, clipping and alpha blending.
- Golden-image tests verify primitive output.
- Runs headlessly in CI on CPU only.

## 2. Complete foundational 2D

**Deliverables**
- Circles and rounded rectangles.
- Paths, fill rules and strokes.
- Multi-stop linear, radial and conic gradients with alpha compositing and transformed clipping.
- Image sampling and filtering.
- Affine transforms, nested clips and off-screen surfaces.
- Defined antialiasing behavior, including coverage-based line rendering.
- Reusable transform and path APIs with fill rules and stroke styles.

**Acceptance criteria**
- Geometry edge cases have tests.
- Pixel output and coordinate conventions are documented.
- Example scenes exercise every supported primitive.

## 3. Text, images and compositor

**Deliverables**
- Image decoding adapters and explicit pixel conversion.
- TTF/OTF font loading, glyph rasterization, text measurement and baseline layout.
- Glyph cache, shaping, fallback fonts and text layout adapter.
- Layer/surface model, opacity, blend modes and transforms.
- Damage-region tracking as an optimization after correct full-frame composition.

**Acceptance criteria**
- Tests cover image formats supported by selected adapters, glyph cache invalidation and composition order.
- A sample UI can combine text, images and multiple layers.
- All external dependencies have reviewed licenses.

## 4. First usable 2D release: common host API and platform adapters

This milestone is part of the 2D release and does not depend on software 3D.

**Deliverables**
- One documented, platform-independent application-facing host API.
- A headless host/surface implementation for deterministic tests and custom
  framebuffer consumers.
- A shared event model for pointer, keyboard, text/IME, resize, scale-factor,
  focus and close events.
- A presentation contract using explicit pixel format, alpha mode, color space,
  dimensions and row stride.
- Separate Windows, Linux and macOS adapter implementations behind the same
  contract; no OS-specific types in application-facing rendering APIs.
- Capability queries and typed unsupported-feature behavior.
- Integration examples that use the same application code on all three OSes.

**Acceptance criteria**
- The same minimal application source builds against the common API on Windows,
  Linux and macOS.
- Each adapter presents CPU-rendered frames and correctly handles resize,
  close, focus, pointer, keyboard and text input where supported.
- Headless tests do not require a window system or display server.
- OS-specific code is isolated and the core renderer builds without platform
  features.
- CI compiles and tests all supported target adapters; real-window smoke tests
  are tracked separately where CI has no display.

**Migration rule:** retain the existing `window` and `native-window` adapters
until the common API covers their required behavior and the examples have been
ported. Do not remove a working integration merely to make the architecture
look cleaner.

## 5. Software 3D foundation

**Deliverables**
- Mesh and vertex/index data structures.
- Camera and transform math.
- Homogeneous clipping, viewport mapping and triangle rasterization.
- Perspective-correct interpolation and depth buffer.
- Texture sampling and a small material/lighting model.

**Acceptance criteria**
- Unit tests cover clipping, depth ordering and interpolation.
- Deterministic test scenes cover a rotating or static 3D object.
- CPU-only CI executes the 3D tests.

## 6. Interactive application frontend and host surfaces

**Deliverables**
- Stable render-target/presentation interfaces.
- Optional Windows/Linux window adapter for presenting GhostEngine-owned CPU frames and collecting input.
- Framebuffer conversion with explicit pixel format and dimensions.
- Interactive browser-style frontend example with toolbar, address field, buttons, text and pointer/keyboard interaction.
- Host-supplied surface adapter and memory-backed framebuffer path.
- Bare-metal integration notes for ArchiaOS.

**Acceptance criteria**
- No desktop window system is required by the headless framebuffer path.
- The browser-style frontend opens on Windows/Linux and displays frames produced by GhostEngine's CPU renderer.
- Mouse and keyboard input reach the application layer.
- Buffer bounds and pixel format conversion are tested.
- UEFI/real-hardware validation is tracked separately from host-side unit tests.

## 4. Hardware acceleration (next major phase after 2D v1)

This phase starts only after GhostEngine 2D v1 is complete. It adds a hardware-accelerated rendering path while keeping the CPU renderer as a supported fallback and correctness reference. GPU work is a separate major phase; it is not a 2D v2 milestone.

**Deliverables**
- A feasibility study for a GhostEngine-owned hardware path.
- A platform and driver compatibility analysis.
- A separate architecture proposal, reviewed before any implementation.

**Acceptance criteria**
- No Vulkan, DirectX, OpenGL or WebGPU dependency is introduced.
- CPU rendering remains fully usable without hardware acceleration.
- No hardware support is claimed until it is implemented and tested on real devices.

## 7. Production readiness

Performance targets and benchmark interpretation are defined in [`docs/PERFORMANCE.md`](PERFORMANCE.md): the CPU renderer targets a 16.67 ms isolated-operation budget on low-end reference systems, while full-scene frame time remains the real 60 FPS acceptance measure. Future GPU targets are separate and must be established with real hardware measurements.

**Deliverables**
- Resource lifetime, memory budgets and cache policies.
- Performance benchmarks and profiling guidance.
- Robust error handling and fuzz/property tests.
- API documentation, examples and migration policy.
- Security review of unsafe code and external input handling.

**Acceptance criteria**
- No known memory-safety or bounds-validation gaps in supported paths.
- Performance is measured against recorded baselines.
- Public APIs have examples and documented limitations.

## 8. Application integrations

Integrate incrementally, with a small demonstrator for each consumer:

- **ArchiaOS:** framebuffer first, then OS compositor/surfaces; GPU acceleration only when the OS driver stack exists.
- **PeaceEmu:** pixel surfaces, texture upload path and frame presentation.
- **Archia Browser:** compositing surfaces, images, text and transforms; browser layout/DOM remain outside GhostEngine.
- **Office suite:** text, vector paths, images, clipping, page rendering and presentation transitions.

**Acceptance criteria**
- Each integration uses the public API rather than private implementation details.
- Integration tests and examples live alongside the relevant adapter or consumer.

## Execution order

1. Repository foundation.
2. Core + software 2D MVP.
3. Foundational 2D.
4. Text/images/compositor.
5. **First usable 2D release:** common host API, headless backend and Windows,
   Linux and macOS adapters.
6. 2D production hardening, cross-platform examples and application integrations.
7. Software 3D (separate milestone, after the 2D release).
8. Optional GPU backend research (deferred).
9. Further consumer integrations and platform expansion.

There is no planned 2D v2: finish and harden the complete 2D v1, then move directly to GPU acceleration, and only after that begin 3D. Do not mark a milestone complete based on scaffolding alone.
