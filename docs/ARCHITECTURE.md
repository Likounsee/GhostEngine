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

The host supplies platform services (for example, a window surface, framebuffer memory, input events or a GPU device). Adapters translate those services into GhostEngine interfaces.

## 2. Layered design

```text
Applications
  ArchiaOS | PeaceEmu | Archia Browser | Office | third-party apps
                         |
                 Public GhostEngine API
                         |
       Core types / resources / frame descriptions
                         |
           Render graph and command validation
                         |
          Backend selection and feature checks
                /                    \
       Software backend          Optional GPU backend
       CPU rasterization         Vulkan (planned)
                \                    /
                 Render targets / output
                         |
           Platform and presentation adapters
                         |
        Host window / framebuffer / display system
```

The diagram describes logical boundaries, not a mandatory runtime dependency on every layer. A small bare-metal application should be able to use core + software rendering + a framebuffer adapter without pulling in desktop windowing or GPU crates.

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
- **platform:** framebuffer descriptors, host surfaces and presentation adapters. Keep platform-specific code isolated.
- **gpu:** optional GPU capability model and backend contract.
- **backend-vulkan:** optional Vulkan implementation. It must remain an optional feature/dependency.

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

## 7. Bare-metal and no_std strategy

Bare-metal support is a design constraint, not an already completed feature.

- Keep core data structures free of operating-system assumptions.
- Evaluate a `no_std`-compatible core and software subset; isolate allocation behind an allocator strategy where practical.
- Do not let desktop windowing, filesystem access, threads or GPU loaders become mandatory dependencies of core.
- Accept a framebuffer descriptor supplied by the boot environment or OS: address, dimensions, stride, pixel format and memory lifetime.
- Validate framebuffer size and arithmetic to prevent out-of-bounds access.
- Initially support a linear framebuffer. Display mode setting, GPU initialization, input and hardware acceleration belong to the host OS/driver stack.

## 8. GPU backend strategy

GPU acceleration is optional. Implement the backend contract only after the software API is stable enough to express useful work.

- Query capabilities explicitly.
- Define which features are supported and which require software fallback.
- Keep GPU-specific handles private to backend modules.
- Do not require DirectX or Windows.
- Vulkan is a candidate backend, not a dependency of the core or software renderer.
- Compare GPU output to software reference scenes within documented tolerances.

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
