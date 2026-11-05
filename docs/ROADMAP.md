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
- Gradients, image sampling and filtering.
- Affine transforms, nested clips and off-screen surfaces.
- Defined antialiasing behavior, including coverage-based line rendering.

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

## 4. Software 3D foundation

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

## 5. Host surfaces and framebuffer output

**Deliverables**
- Stable render-target/presentation interfaces.
- Host-supplied surface adapter.
- Linear framebuffer adapter with explicit stride and pixel format.
- Integration example using a memory-backed framebuffer.
- Bare-metal integration notes for ArchiaOS.

**Acceptance criteria**
- No desktop window system is required by the framebuffer path.
- Buffer bounds and pixel format conversion are tested.
- UEFI/real-hardware validation is tracked separately from host-side unit tests.

## 6. Hardware acceleration research (deferred)

This is not part of the initial implementation plan. The CPU renderer remains the complete reference implementation and must not depend on an external graphics API.

**Possible future deliverables**
- A feasibility study for a GhostEngine-owned hardware path.
- A platform and driver compatibility analysis.
- A separate architecture proposal, reviewed before any implementation.

**Acceptance criteria**
- No Vulkan, DirectX, OpenGL or WebGPU dependency is introduced.
- CPU rendering remains fully usable without hardware acceleration.
- No hardware support is claimed until it is implemented and tested on real devices.

## 7. Production readiness

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
5. Software 3D.
6. Framebuffer/platform adapters.
7. Optional GPU backend.
8. Hardening and consumer integrations.

Do not start GPU acceleration before the software renderer and public resource model are usable. Do not mark a milestone complete based on scaffolding alone.
