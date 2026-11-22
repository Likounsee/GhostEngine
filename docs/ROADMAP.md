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

## 5. 2D v1 integration and production hardening

This is the final 2D v1 milestone. It closes the gap between the renderer,
the common host contract and real application integrations.

**Deliverables**
- A stable render-target and presentation interface with explicit pixel
  format, alpha mode, color space, dimensions and row stride.
- Windows, Linux and macOS host adapters implementing the same public API.
- A shared application example using the common API on all three platforms.
- An interactive browser-style frontend example with toolbar, address field,
  buttons, text and pointer/keyboard interaction.
- Host-supplied surfaces and memory-backed framebuffer paths.
- Resource lifetime, memory budgets, cache policies and robust error handling.
- Property/fuzz tests for geometry, clipping, image decoding and buffer bounds.
- API documentation, examples, migration policy and security review.

**Acceptance criteria**
- Same minimal application source builds against the common API on Windows,
  Linux and macOS.
- Each adapter handles lifecycle, resize, DPI, focus, pointer, keyboard and
  text input/IME wherever supported, and presents CPU-rendered frames.
- Headless rendering remains usable without a window system.
- CI compiles and tests supported target adapters; real-window smoke tests are
  tracked separately where CI has no display.
- No known bounds-validation or memory-safety gaps remain in supported paths.
- Performance is measured against recorded baselines; no optimization is
  declared successful without before/after measurements.

## 6. Hardware acceleration (next major phase after 2D v1)

This phase starts only after all 2D v1 acceptance criteria are met. It adds a
hardware-accelerated rendering path while keeping the CPU renderer as a
supported fallback and correctness reference. This is not a 2D v2 milestone.

**Deliverables**
- A feasibility and architecture study for a GhostEngine-owned GPU path.
- Platform and driver compatibility analysis for Windows, Linux and macOS.
- Device discovery, capability reporting and deterministic CPU fallback.
- Resource, command submission, synchronization, lifetime and device-loss
  design before backend implementation.
- GPU implementation and real-device validation, selected after the study.

**Acceptance criteria**
- CPU rendering remains fully usable without hardware acceleration.
- GPU and CPU outputs are compared with deterministic correctness tests.
- No hardware support is claimed until implemented and tested on real devices.
- Performance claims use repeatable benchmarks on documented hardware.

## 7. 3D rendering (only after GPU acceleration)

3D development starts after the GPU phase has a working, validated foundation.
It must reuse the established resource and submission architecture rather than
introducing a second unrelated backend.

**Deliverables**
- Mesh and vertex/index data structures.
- Camera and transform math.
- Homogeneous clipping, viewport mapping and triangle rasterization.
- Perspective-correct interpolation and depth buffer.
- Texture sampling and a small material/lighting model.

**Acceptance criteria**
- Unit tests cover clipping, depth ordering and interpolation.
- Deterministic scenes cover a rotating or static 3D object.
- CPU fallback and supported GPU backends remain consistent where practical.

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
