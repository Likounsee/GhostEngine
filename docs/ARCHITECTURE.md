# GhostEngine — Architecture

**Status:** proposed architecture, to be validated during implementation.

## 1. Scope and non-goals

GhostEngine is a reusable rendering engine. It owns rendering algorithms, frame/resource abstractions, the CPU renderer and composition logic.

It does not initially aim to:
- implement a complete window manager, input stack or display driver;
- replace GPU vendor drivers or provide direct access to every GPU;
- require a GPU, Vulkan, OpenGL, DirectX or a desktop operating system;
- expose backend-specific objects as the primary public API;
- implement every advanced 3D feature before a useful 2D renderer exists.

The host supplies platform services (for example, a window surface, framebuffer memory or input events). Adapters translate those services into GhostEngine interfaces.

## 2. Layered design

```text
Applications
  ArchiaOS | PeaceEmu | Archia Browser | Office | third-party apps
                         |
                 Public GhostEngine API
                         |
       Core types / resources / frame descriptions
                         |
              CPU software renderer
        Rasterization / sampling / blending
                         |
                 Render targets / output
                         |
           Platform and presentation adapters
                         |
        Host window / framebuffer / display system
```

The diagram describes logical boundaries, not a mandatory runtime dependency on every layer. Headless applications can use core + software rendering without pulling in windowing or platform crates.

## 3. Workspace and crate boundaries

Create crates only when their API is justified. The following is the intended eventual split:

- **core:** identifiers, typed handles, resource descriptors, pixel formats, color types, rectangles, transforms, errors, capability descriptions and backend-independent traits.
- **render:** frame lifecycle, command recording, render-pass descriptions, resource dependency validation and execution dispatch.
- **software:** CPU-owned surfaces, rasterization, sampling, clipping, blending, depth and software execution.
- **2d:** paths, tessellation or scan conversion, strokes, fills, gradients, image drawing and 2D state.
- **3d:** mesh data, camera, transforms, clipping, triangle setup, depth, interpolation, materials and lighting.
- **compositor:** ordered surfaces/layers, damage regions, transforms, opacity and composition.
- **text:** TTF/OTF loading, bounded glyph caches, OpenType GSUB/GPOS shaping, bidi run ordering, font fallback and baseline-based rasterization. Display-list text is shaped during recording; low-level framebuffer APIs retain a simple glyph-by-glyph path.
- **image:** image format adapters, decoded pixel storage and texture descriptors. Decoders should be replaceable.
- **platform:** host surfaces and presentation adapters. Keep platform-specific code isolated.
- No graphics-API backend crate is part of the current design. Any future hardware-acceleration work requires a separate architecture decision and must not introduce Vulkan, DirectX, OpenGL or WebGPU as a rendering dependency.

Do not create empty crates just to match this list. Start with a compact workspace and split once code ownership is clear.

## 3.1 Common application API and per-OS platform backends

**Architecture decision for the first usable 2D release:** applications use one
GhostEngine host API. Windows, Linux and macOS are three implementations behind
that API, not three incompatible public APIs that every application must learn.

The dependency direction is:

```text
Application (browser, emulator, UI, ...)
              |
      GhostEngine common API
      |                    |
  2D renderer         Host/platform contract
  CPU framebuffer     window, events, clipboard,
  display lists       presentation surface
                           |
                 platform implementation
                  /         |         \
             Windows      Linux      macOS
```

Rules:

- The 2D renderer, framebuffer, geometry, display lists, resources and input
  event types remain platform-independent.
- The common host API owns the lifecycle visible to applications: create/close
  a window, query drawable size and scale, receive normalized input/text events,
  request redraw, present a frame, and access optional clipboard capabilities.
- Platform implementations translate native OS events and presentation
  requirements into those common types. Native handles and OS-specific types
  must not leak into the renderer or ordinary application code.
- Presentation is a boundary: the CPU renderer produces pixels; the platform
  backend validates the pixel format/stride and transfers the frame to the
  native surface. A backend must not silently change rendering semantics.
- Capabilities that differ by OS (clipboard, IME, display scaling, window
  decorations, protected surfaces, etc.) are queried explicitly. Unsupported
  features return a typed result; they are never silently emulated as success.
  Native window startup must not depend on optional clipboard initialization:
  clipboard requests fail explicitly when the current desktop session cannot
  provide that service.
- A headless backend remains available for tests, servers, image generation and
  applications that provide their own surface.
- Backend selection is internal to the common API (compile-time target
  selection or an explicit backend choice where useful). Applications should
  not need separate Windows/Linux/macOS rendering code.

**Implementation timing:** define and test the common contract during 2D
finalization, because the existing framebuffer and input types are already
platform-independent. Implement and validate the Windows, Linux and macOS
adapters incrementally. Do not create three empty crates or duplicate the
renderer now. Keep the current window adapters as transitional integration
paths until the common host API can replace them without breaking examples.

The platform-neutral contract now exists in `host.rs` and is re-exported from
the crate root. It defines validated window configuration, backend creation and
closure, normalized lifecycle/resize/focus/redraw/input events, capability
reporting, physical drawable sizing, scale-factor reporting and framebuffer
presentation. `HeadlessHost` is the first implementation and provides
repeatable, display-free lifecycle, event, resize and presentation behavior,
including zero-sized minimized drawables and an explicit presentation error
until the drawable is restored.
`SurfaceDescriptor` and `Framebuffer::write_to_surface` remain the explicit
pixel-memory boundary. The native Winit adapter remains callback-driven rather than implementing the
poll-based trait. It normalizes lifecycle/input events, synchronizes physical
framebuffer dimensions on resize and DPI transitions, and tracks minimized
zero-sized drawables separately from retained framebuffer storage so restoring
a window always emits a fresh resize event. Native Windows, Linux and macOS
behavior still requires platform CI and interactive testing.

## 4. Core API rules

- Use explicit pixel formats, color spaces and alpha conventions. Document whether colors are linear or sRGB at each boundary.
- Use typed resource handles with generation/lifetime validation rather than exposing raw pointers.
- Define resource ownership and synchronization rules before adding asynchronous execution.
- Make dimensions, strides, alignment and buffer lengths explicit for external memory.
- Keep backend selection and feature queries separate from drawing commands.
- Return structured errors; avoid panics for invalid user-provided render descriptions.
- Define fallback behavior when a backend does not support a requested feature.
- Keep unsafe code minimal, isolated and documented.

## 5. Software renderer

The software renderer is the reference backend and the first real implementation target.

### Initial 2D pipeline
1. Validate target dimensions, format and buffer layout.
2. Clear or load the target.
3. Transform primitive coordinates into target space.
4. Clip geometry to the target and active clip region.
5. Rasterize primitives into pixels.
6. Apply documented blending and coverage rules.
7. Produce a stable output buffer suitable for tests or presentation.

Start with solid-color rectangles, lines and triangles, then add circles, paths, gradients, images, filtering and anti-aliasing. Avoid claiming full vector compatibility until edge cases and tests exist.

### 3D pipeline
3D is intentionally deferred until after GhostEngine 2D V1 is complete and hardware acceleration has a working, validated foundation. Do not start a separate software-3D milestone between 2D V1 and GPU acceleration. The eventual 3D pipeline will reuse the GPU resource and command architecture:
1. Vertex transformation.
2. Homogeneous clipping.
3. Perspective divide and viewport mapping.
4. Triangle setup and rasterization.
5. Perspective-correct varying interpolation.
6. Depth test/write.
7. Texture sampling and material evaluation.
8. Basic lighting.

A programmable shader language is a later design decision. Begin with a small, explicit material model rather than promising arbitrary GPU shader compatibility.

## 6. Resource and memory model

- Distinguish CPU pixel storage from backend-owned GPU resources.
- Describe external memory with pointer/length/stride/format contracts and strict lifetime requirements.
- Use explicit create/update/destroy operations or RAII-owned handles.
- Avoid hidden global resource state.
- Keep explicit memory budgets and eviction policies for texture, path and glyph caches; avoid unbounded growth in long-running applications.
- Treat device loss and backend reset as GPU-backend concerns; core resources should be recreatable where feasible.

## 7. Windows, Linux and macOS; ArchiaOS later

The desktop platform adapters target Windows, Linux and macOS. Keep operating-system APIs isolated from the rendering core and avoid pulling windowing dependencies into headless builds.

ArchiaOS integration is deferred until that operating system is ready. A future port may reuse the CPU renderer through a host-supplied framebuffer or surface descriptor. A no_std-compatible subset and bare-metal-specific integration are future investigations, not current implementation requirements.

## 8. Hardware acceleration follows 2D V1

The current implementation remains a CPU software renderer and must build without Vulkan, DirectX, OpenGL, WebGPU or a GPU. Once 2D V1 acceptance criteria are met, hardware acceleration becomes the next major development phase (there is no 2D V2 phase).

That phase requires a feasibility study, a GhostEngine-owned backend/resource architecture, explicit device capabilities and fallback behavior, synchronization and resource-lifetime rules, device-loss handling, real-device correctness tests and repeatable end-to-end benchmarks. The CPU renderer remains a fully supported fallback and correctness reference. Do not claim GPU support or compatibility before it is implemented and tested on real devices.

3D development begins only after the hardware-accelerated foundation is working and validated. It reuses that backend and resource architecture rather than introducing an unrelated renderer.


The common synchronous `HostBackend` contract is currently implemented by
the deterministic headless backend. The native Winit adapter provides an
application-facing callback runner through `NativeWindowSurface::run_with_config`
and `run_with_config_control`. It delivers normalized lifecycle/input events,
owns event-loop dispatch, presents the CPU framebuffer and supports wait,
continuous-redraw and exit outcomes. It deliberately does not implement the
poll-based `HostBackend` trait: Winit owns a callback-driven event loop, and the
engine must not pretend it can synchronously poll that loop from an arbitrary
caller thread. Keep both lifecycle models explicit rather than hiding the
difference behind a blocking or synthetic polling abstraction.

## 9. Testing strategy

- Unit tests for math, clipping, format conversion, blending and resource validation.
- Pixel tests using tiny known scenes and golden buffers.
- Property/fuzz tests for dimensions, strides, clipping bounds and malformed commands.
- Integration tests that execute the same supported scene through software and GPU backends.
- CI must run the software suite without a GPU, display server or network access.
- Benchmarks should report scene, resolution, hardware and build profile.

## 10. Dependency and licensing policy

Before adding a dependency, record its purpose, license, transitive-license considerations, maintenance status and whether it is optional. Prefer MIT, Apache-2.0 and BSD-compatible components. A dependency may assist with platform integration, parsing or acceleration, but must not replace GhostEngine's core renderer or make a proprietary product mandatory.

## 11. Open decisions

Resolve these through small prototypes and documented decisions rather than speculative abstraction:
- exact public API shape and stability policy;
- math crate versus internal math types;
- remaining advanced text-layout gaps, full Unicode line breaking and broader font fallback policy;
- image decoder selection;
- threading and job scheduling model;
- color management and HDR scope;
- shader/material model;
- optional GPU backend order;
- precise `no_std` feature boundary.
