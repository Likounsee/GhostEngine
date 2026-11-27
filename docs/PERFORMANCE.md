# GhostEngine — Performance targets

## CPU software renderer

GhostEngine's CPU renderer is a first-class backend, not a temporary fallback. Its performance targets must be realistic on small systems and must not assume a dedicated graphics processor.

- **Baseline target:** each representative raster operation should aim to complete within **16.67 ms** on the low-end reference machine, equivalent to 60 operations/frames per second when measured in isolation.
- **Scene budget:** a real frame contains multiple operations. The total CPU render workload, including composition and presentation overhead, must fit within the frame budget; individual benchmark rows are diagnostic and must not each be budgeted the entire frame simultaneously.
- **Quality:** performance work must preserve documented sampling, alpha-compositing, clipping and transform behavior. A faster result is not acceptable if it introduces visible artifacts or changes output unexpectedly.
- **Scalability:** benchmark single-threaded and parallel paths separately. Parallel execution is an optimization, not a requirement for small workloads where thread overhead can dominate.
- **Memory:** reusable scratch buffers and caches should have explicit bounds and predictable invalidation behavior.

The current CPU benchmark uses a 1920x1080 destination and includes both small UI-like work and large raster workloads. Its 16.67 ms indicator is a per-operation diagnostic threshold, not a universal promise for every machine or every scene. Results must always be interpreted alongside the CPU model, resolution, build mode and benchmark protocol.

## Recorded CI baseline — 2026-11-23

A release benchmark was completed on all three GitHub-hosted runner platforms
(workflow run [45207786406](https://github.com/Likounsee/GhostEngine/actions/runs/45207786406),
source revision `6ee07257`). All three used Rust `rustc 1.100.0`, Release mode,
SIMD enabled and parallel rendering enabled. Each runner exposed only 3–4
logical CPUs, so these numbers are a reproducible CI snapshot, not a low-end
desktop reference or a cross-machine performance guarantee.

| Workload | Windows x86_64 | Linux x86_64 | macOS arm64 |
|---|---:|---:|---:|
| Linear gradient | 11.81 ms | 11.76 ms | 11.05 ms |
| Radial gradient | 12.08 ms | 12.04 ms | 15.13 ms |
| Texture bilinear | 9.17 ms | 11.19 ms | 4.86 ms |
| Antialiased UI button + text | 12.46 ms | 14.01 ms | 11.23 ms |
| Nested groups, reused scratch (uncached) | 26.33 ms | 28.83 ms | 26.00 ms |
| Nested groups, persistent cache | 9.19 ms | 9.03 ms | 9.83 ms |

| Runner metadata | Windows x86_64 | Linux x86_64 | macOS arm64 |
|---|---|---|---|
| Reported CPU | AMD64 Family 25 Model 17 Stepping 1 | AMD EPYC 9V74 | Apple M1 (Virtual) |
| Logical CPUs | 4 | 4 | 3 |

The benchmark also measures a partially invalidated persistent group. This path includes damage-list traversal, off-screen partial regeneration, and copying only dirty rows back into the retained surface; it should be tracked separately from the clean-cache composition path.

The nested-group result is the clearest optimization signal: rebuilding every
group surface each frame exceeds the isolated 16.67 ms diagnostic target on
these virtual runners, while retaining stable group IDs and using
`LayerGroupCache` keeps the measured composition below 10 ms. Prefer the
persistent cache for mostly static group content and mark local damage when
content changes. The uncached path remains useful for dynamic scenes and as a
simple correctness/reference path.

Mipmap construction for the 3840x2160 source took 50.7–139.7 ms as one-time
setup on these runners; steady-state 4K-to-1080p mipmapped drawing measured
0.18–0.46 ms per frame. Keep these setup and frame costs separate when
evaluating the workload.

## GPU backend (future work)

A future GPU backend should target substantially lower rendering latency and higher throughput where supported by the hardware. Its targets must be defined separately after selecting an API and measuring real devices. Do not compare CPU and GPU numbers without accounting for upload, synchronization, readback, presentation and workload differences.

GPU acceleration remains deferred until the CPU renderer, public resource model and backend boundaries are stable. The CPU renderer must remain fully usable and must continue to serve as a correctness reference.

## Benchmark interpretation

- Compare runs made with the same resolution, workload, build mode and iteration count.
- Use repeated runs; small CPU systems are sensitive to background load and thermal/power limits.
- Track regressions as well as improvements. A result below 16.67 ms is within the isolated-operation target, not proof that a complete application scene sustains 60 FPS.
- Keep setup costs (for example mipmap construction) separate from steady-state frame timings and report them explicitly.
