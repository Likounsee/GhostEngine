# GhostEngine 2D V1 — release checklist

This checklist is the final gate for a versioned 2D V1 release. It deliberately excludes GPU acceleration, 3D and consumer-application migration (including Amethys Launcher).

## 1. Source and automated validation

- [x] Freeze code candidate commit `64cae9212bd5872e174137b24db0d277e57bef00` and record its full SHA.
- [x] Obtain a completed, green GitHub Actions matrix for that exact SHA on Windows, Linux and macOS (run `46181055239`); Rust 1.87 also passes.
- [x] Confirm formatting, Clippy with warnings denied and public API docs with warnings denied.
- [x] Confirm scalar fallback, SIMD, parallel rendering, native-input, native-window and all-feature test configurations.
- [x] Confirm all headless examples, image decoding/mipmap showcase, Amethys integration and all 18 scripted PNG frames pass on Windows, Linux and macOS.
- [x] Confirm no known critical correctness, memory-safety, lifecycle or input defects remain in the audited source and automated tests; physical desktop validation remains open.
- [x] Review the public API for accidental unstable/internal exports and document intentional compatibility limits.

## 2. Real desktop validation (required)

Complete one [desktop validation record](PLATFORM_VALIDATION.md) for each supported OS: Windows, Linux and macOS. Test the native UI and paced animation examples interactively, including resize, DPI, minimize/restore, focus, pointer, keyboard, text, IME, clipboard and clean shutdown.

CI compilation is not a substitute for physical desktop testing. If a capability is unavailable on a particular desktop session, document the limitation and verify that the API reports it explicitly rather than silently claiming success.

## 3. Reliability and resource bounds

- [x] Run repeated insert/evict/invalidate cycles for texture, path, glyph and layer caches; record memory behavior and stale-handle outcomes (unified cache lifecycle stress test passes the cross-platform CI matrix).
- [ ] Run repeated native window resize/minimize/restore/present/close cycles on each physical OS desktop.
- [x] Audit allocation-bearing raster scratch paths and compatibility wrappers; add fallible polygon, rectangle and triangle antialiased-outline APIs, reserve required buffers before framebuffer mutation, export the APIs and cover them with equivalence tests. Physical desktop validation remains separate.
- [x] Keep deterministic pixel/golden tests for representative optimized rendering paths and cache/composition behavior.
- [x] Record known API/platform limitations and validation boundaries in the public docs; platform-specific physical results remain to be filled in.

## 4. Performance evidence

- [ ] Run release benchmarks on reference desktop hardware for Windows, Linux and macOS.
- [x] Record the latest three-platform CI benchmark metadata and mixed-scene results in [PERFORMANCE.md](PERFORMANCE.md); physical reference hardware records remain open.
- [x] Profile a representative mixed scene (text + UI + images + layers) on all three CI platforms; see [PERFORMANCE.md](PERFORMANCE.md).
- [x] Compare CI results against the existing [performance baseline](PERFORMANCE.md), identify the 4K fractional trilinear hotspot and preserve correctness tests; reference desktop comparison remains open.
- [x] Keep virtual CI results explicitly separate from physical desktop benchmarks; do not claim universal 60 FPS.

## 5. Documentation and licensing

- [x] Verify headless example commands and required Cargo features in CI; interactive native examples still need desktop runs.
- [ ] Document supported capabilities and known per-platform limitations from actual physical validation.
- [ ] Review dependency licenses and transitive notices from the exact release lockfile; include required notices in the release archive.
- [x] Verify bundled font/image fixtures are clearly identified as test assets and their licenses/attributions are included.
- [x] Review README, FEATURES, ARCHITECTURE, API docs and migration guidance for consistency.
- [x] Confirm MIT and Apache-2.0 license files are present.

For a dependency-license audit, run from `ghostengine/` with a current Rust toolchain and a license-audit tool approved by the maintainer. Preserve the full output alongside the release candidate; do not infer dependency licenses from crate names.

## 6. Versioned release

- [ ] Choose the version only after sections 1–5 are satisfied.
- [ ] Update the crate version and release notes together.
- [ ] Run `cargo package --workspace --list` and inspect the package contents.
- [ ] Run `cargo package --workspace --locked` and resolve missing files/metadata.
- [ ] Build and test the packaged crate, not only the repository workspace.
- [ ] Tag the exact validated commit and publish the source/archive with release notes and notices.

## Current boundary

Amethys Launcher integration, GPU acceleration and 3D are not release blockers for GhostEngine 2D V1. A test-only application fixture is not evidence of a production consumer integration.
