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
- **text:** text runs, layout integration, glyph atlas/cache and text drawing. Font parsing/shaping dependencies must be license-reviewed.
- **image:** image format adapters, decoded pixel storage and texture descriptors. Decoders should be replaceable.
- **platform:** host surfaces and presentation adapters. Keep platform-specific code isolated.
- No graphics-API backend crate is part of the current design. Any future hardware-acceleration work requires a separate architecture decision and must not introduce Vulkan, DirectX, OpenGL or WebGPU as a rendering dependency.

Do not create empty crates just to match this list. Start with a compact workspace and split once code ownership is clear.

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
Add a software 3D pipeline only after the 2D target and resource model are stable:
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
- Add budgets and eviction policies only when texture/font caches exist.
- Treat device loss and backend reset as GPU-backend concerns; core resources should be recreatable where feasible.

## 7. Windows and Linux first; ArchiaOS later

The first platform adapters target Windows and Linux. Keep operating-system APIs isolated from the rendering core and avoid pulling windowing dependencies into headless builds.

ArchiaOS integration is deferred until that operating system is ready. A future port may reuse the CPU renderer through a host-supplied framebuffer or surface descriptor. A no_std-compatible subset and bare-metal-specific integration are future investigations, not current implementation requirements.

## 8. Hardware acceleration is out of initial scope

The initial engine is a CPU software renderer. It must not depend on Vulkan, DirectX, OpenGL, WebGPU or an existing rendering engine.

If hardware acceleration is reconsidered later, it requires a separate feasibility study and an independently designed GhostEngine-owned architecture. It must not compromise the portability, testability or independence of the CPU renderer. No hardware backend or driver compatibility is claimed by the current project.

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
- font shaping and rasterization integration;
- image decoder selection;
- threading and job scheduling model;
- color management and HDR scope;
- shader/material model;
- optional GPU backend order;
- precise `no_std` feature boundary.
