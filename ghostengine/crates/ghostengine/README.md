# GhostEngine

GhostEngine is an independent Rust 2D rendering engine. This crate provides a CPU rasterizer, vector paths, text shaping and rasterization, textures, composition, retained UI, and host adapters.

The renderer produces its own CPU framebuffer. The optional native window feature presents that framebuffer; it does not replace the renderer with a platform graphics API.

## Minimum supported Rust version

Rust 1.87.

## Feature flags

- `simd`: SIMD acceleration for supported CPU raster operations.
- `parallel-rendering`: parallel CPU rendering paths.
- `image-decoding`: BMP, JPEG, PNG, and PNM decoding.
- `native-input`: native keyboard, text/IME, and clipboard integration.
- `native-window`: Winit windowing and softbuffer framebuffer presentation; enables `native-input`.
- `window`: legacy minifb window adapter.

The default features enable `simd` and `parallel-rendering`.

## Examples

Run these commands from the `ghostengine/` workspace directory:

```sh
cargo test --workspace --all-targets --all-features
cargo run -p ghostengine --release --example first_image -- first-image.ppm
cargo run -p ghostengine --release --example headless_host_demo
cargo run -p ghostengine --release --example vector_path_showcase -- vector-showcase.ppm
cargo run -p ghostengine --release --features image-decoding --example image_pipeline_demo -- input.png output.ppm
cargo run -p ghostengine --release --features native-window --example native_ui_demo
cargo run -p ghostengine --release --features native-window --example native_animation_demo
```

The two native examples open interactive desktop windows and require a working windowing session.

## License

Licensed under either of:

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at your option.
