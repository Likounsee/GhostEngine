# GhostEngine 2D V1 — release checklist

This checklist is the final gate for a versioned 2D V1 release. It deliberately excludes GPU acceleration, 3D and consumer-application migration (including Amethys Launcher).

## 1. Source and automated validation

- [ ] Freeze the candidate commit and record its full SHA.
- [ ] Obtain a completed, green GitHub Actions matrix for that exact SHA on Windows, Linux and macOS.
- [ ] Confirm formatting, Clippy with warnings denied and public API docs with warnings denied.
- [ ] Confirm scalar fallback, SIMD, parallel rendering, native-input, native-window and all-feature test configurations.
- [ ] Confirm all examples and the image-decoding showcase build and run in their supported headless/desktop mode.
- [ ] Confirm no known critical correctness, memory-safety, lifecycle or input defects remain.
- [ ] Review the public API for accidental unstable/internal exports and document intentional compatibility limits.

## 2. Real desktop validation (required)

Complete one [desktop validation record](PLATFORM_VALIDATION.md) for each supported OS: Windows, Linux and macOS. Test the native UI and paced animation examples interactively, including resize, DPI, minimize/restore, focus, pointer, keyboard, text, IME, clipboard and clean shutdown.

CI compilation is not a substitute for physical desktop testing. If a capability is unavailable on a particular desktop session, document the limitation and verify that the API reports it explicitly rather than silently claiming success.

## 3. Reliability and resource bounds

- [ ] Run repeated insert/evict/invalidate cycles for texture, path, glyph and layer caches; record memory behavior and stale-handle outcomes.
- [ ] Run repeated window resize/minimize/restore/present/close cycles on each OS.
- [ ] Finish reviewing infallible convenience APIs and internal raster scratch allocations; invalid external input must return errors rather than panic or write out of bounds.
- [ ] Keep deterministic pixel/golden tests for every optimized rendering path.
- [ ] Record any accepted limitations and ensure they are reflected in the public docs.

## 4. Performance evidence

- [ ] Run release benchmarks on reference desktop hardware for Windows, Linux and macOS.
- [ ] Record commit SHA, CPU, OS, Rust version, feature set, resolution and iteration count.
- [ ] Profile representative mixed scenes (text + UI + images + layers), not only isolated raster operations.
- [ ] Compare against the existing [performance baseline](PERFORMANCE.md); explain regressions and retain correctness equivalence.
- [ ] Do not present virtual CI runner results as physical desktop benchmarks or claim universal 60 FPS.

## 5. Documentation and licensing

- [ ] Verify every public example's command and required Cargo feature.
- [ ] Document supported capabilities and known per-platform limitations from actual validation.
- [ ] Review dependency licenses and transitive notices from the exact release lockfile; include required notices in the release archive.
- [ ] Verify bundled font/image fixtures are clearly identified as test assets and their licenses/attributions are included.
- [ ] Review README, FEATURES, ARCHITECTURE, API docs and migration guidance for consistency.
- [ ] Confirm MIT and Apache-2.0 license files are present.

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
