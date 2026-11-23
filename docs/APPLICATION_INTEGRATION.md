# GhostEngine — application integration status

This document records what has actually been exercised through GhostEngine's public API.
A compiling example is useful evidence, but it is not equivalent to running a production
application or validating a native desktop session.

## Public application workflow covered by automated tests

The integration tests exercise a complete display-free application path:

1. Create a retained component tree using public `UiComponent`, `ComponentTree`,
   `StackLayout` and `LayoutItem` APIs.
2. Compute layout and route pointer press/release events to a button.
3. Verify the activated component ID and keyboard activation of the focused control.
4. Render the retained UI and text into a CPU `Framebuffer` with a licensed font fixture.
5. Present the frame through the public `HostBackend` implementation (`HeadlessHost`).
6. Export the presented frame through `SurfaceDescriptor`, including pixel format,
   row stride, alpha mode and color space.

The end-to-end regression is
`ghostengine/crates/ghostengine/tests/host_pipeline.rs`,
`public_application_workflow_routes_ui_input_renders_text_and_presents_frame`.

Other examples cover:
- `headless_host_demo`: display-free host creation, events and presentation.
- `native_ui_demo`: retained UI, keyboard/text input, IME and clipboard on the native host.
- `native_animation_demo`: paced native redraw without a busy loop.
- `browser_frontend`: an interactive frontend-shaped example using the transitional minifb adapter.
- `vector_path_showcase`: headless paths, curves, transforms, fill rules, strokes and clipping.
- `image_pipeline_demo`: headless BMP/JPEG/PNG/PNM decoding, nearest/bilinear sampling, mipmap generation and clipped mipmapped drawing.
- `first_image`: basic raster primitives and image export.

## Amethys Launcher prototype — scope and limits

`Amethys-Launcher test/` is a **test-only, standalone prototype inspired by Amethys Launcher**.
Its generated PNG cover art, bundled sample fonts and fictional game/store data are fixtures.
It is not the production Amethys Launcher repository and must never be described as a completed
production integration.

The prototype is useful for checking a denser app-like scene: text, cards, images, mipmaps,
texture caching, scrolling, search, filters, navigation and scripted frame rendering.
Its headless scripted session and 18 PNG outputs run in CI on Windows, Linux and macOS.

What this does prove:
- the exercised public CPU-rendering APIs compose into a non-trivial interface;
- the prototype can build frames and handle scripted interactions without a display server;
- the exercised image, texture-cache and mipmap paths work with its fixture assets.

What this does **not** prove:
- production Amethys Launcher source code compiles against GhostEngine;
- production launcher behavior, real game discovery, real cover sources or real store data work;
- native windows, DPI scaling, IME, clipboard and interactive input have been manually validated
  on physical Windows, Linux and macOS desktop sessions.

## Scope boundary and remaining desktop validation

Consumer application migration is intentionally **not part of GhostEngine 2D V1**. In particular,
Metis Launcher integration is a separate follow-up owned by the consumer project; this engine task
only delivers and validates the reusable 2D rendering, UI, input and host APIs. The Amethys fixture
prototype above remains a test fixture, not a production integration target.

The remaining release-validation task is to run the native examples interactively on real desktop
sessions and record OS, backend, scale factor, resize/minimize/restore, focus, pointer, keyboard,
text/IME and clipboard results. CI currently compiles and tests the native adapters on Windows,
Linux and macOS, but does not prove physical desktop behavior.

## CI evidence

The GhostEngine workflow runs formatting, Clippy, public API documentation, scalar/SIMD/parallel
feature configurations, native-input and native-window feature configurations, all-feature tests,
the vector showcase, the Amethys prototype integration tests, its scripted headless session and
the 18-image output check on Windows, Linux and macOS.

A green CI run confirms those automated checks only. It does not prove interactive native-window behavior on physical desktops. Production consumer integration, including Metis Launcher, is deliberately deferred and is not required to finish GhostEngine 2D V1.