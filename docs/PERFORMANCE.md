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

Persistent group-cache eviction is also part of the measured memory-management
path. Budget reductions and cache growth use one sorted LRU candidate list rather
than rescanning the complete cache to find each next victim. Candidate storage
and reclaimable capacity are validated before eviction begins, so a failed
budget change does not partially empty the cache. Large cache trims therefore
use O(n log n) ordering rather than repeated O(n) minimum searches.

When the cache is already at its entry limit, inserting a new key first reuses the table capacity released by the LRU victim. It does not reserve an additional hash-table slot before eviction, which previously caused avoidable table growth on full-cache replacement and made retained-memory accounting rise despite a fixed entry limit. Non-full insertions still reserve before mutation so allocation failure leaves existing entries intact.

The path geometry cache keeps an exact running total of retained flattened-
geometry payload bytes. Cache hits no longer require a full scan to report
usage, and invalidation/clear operations update the counter directly. When an
insertion or policy change needs multiple evictions, entries are ordered once
by recency and the oldest prefix is removed as a batch. This avoids repeated
minimum searches and full-cache byte recounts. The CPU benchmark includes both
a steady-state path-cache hit workload and a bounded-cache churn workload.

The texture cache uses the same batched eviction strategy for both budget
reductions and insertions that need to reclaim several resources. Retained
textures are excluded from the candidate list, ties are deterministic by
handle, and failed budget reductions preserve the previous entries, counters
and configured budget.

The nested-group result is the clearest optimization signal: rebuilding every
group surface each frame exceeds the isolated 16.67 ms diagnostic target on
these virtual runners, while retaining stable group IDs and using
`LayerGroupCache` keeps the measured composition below 10 ms. Prefer the
persistent cache for mostly static group content and mark local damage when
content changes. The uncached path remains useful for dynamic scenes and as a
simple correctness/reference path.

Mipmap construction for the 3840x2160 source took 50.7–139.7 ms as one-time
setup on these runners; steady-state 4K-to-1080p mipmapped drawing measured
0.18–0.46 ms per frame. The benchmark now also includes a 4K-to-1440x810
fractional-LOD workload so the general trilinear sampler is measured separately
from the exact 2:1 mip-row copy path. The general sampler uses precomputed
reciprocal mip scales and direct row-major texel indexing after coordinate
clamping, avoiding repeated scale divisions and checked pixel lookups in the
inner sampling loop. Keep setup and frame costs separate when evaluating the workload.

## GPU backend (future work)

A future GPU backend should target substantially lower rendering latency and higher throughput where supported by the hardware. Its targets must be defined separately after selecting an API and measuring real devices. Do not compare CPU and GPU numbers without accounting for upload, synchronization, readback, presentation and workload differences.

GPU acceleration remains deferred until the CPU renderer, public resource model and backend boundaries are stable. The CPU renderer must remain fully usable and must continue to serve as a correctness reference.

## Benchmark interpretation

- Compare runs made with the same resolution, workload, build mode and iteration count.
- Use repeated runs; small CPU systems are sensitive to background load and thermal/power limits.
- Track regressions as well as improvements. A result below 16.67 ms is within the isolated-operation target, not proof that a complete application scene sustains 60 FPS.
- Keep setup costs (for example mipmap construction) separate from steady-state frame timings and report them explicitly.

## Display-list replay culling

Display-list replay intersects each recorded command clip with the target
framebuffer before dispatch. Commands whose recorded clip cannot touch the
target are counted as skipped without entering path flattening, stroke
construction, text rasterization, gradient evaluation, or texture sampling.
Full-frame clears remain unconditional because they intentionally ignore clip
state. Visible commands continue through their existing clipping and pixel
paths, preserving rendering semantics.

## Damage-clipped transformed layers

Damage-limited layer composition now clips transformed layer bounds against
the active damage rectangle during preparation. Layers outside that rectangle
are rejected before calculating the inverse transform, and intersecting layers
carry the narrower bounds into the scanline compositor. Full-frame and
parallel composition retain the original preparation path. Pixel blending and
sampling are unchanged; the existing transformed-damage parity test covers
the clipped output.


## Reusable display-list stroke workspace

The persistent `DisplayListRenderer` now retains the CPU stroke rasterizer's
geometry buffers as well as texture-sampling buffers. Repeated `StrokePath`
and `StrokeRect` commands reuse flattened contours, transformed points,
segments, joins and per-row work arrays across frames. Both antialiased and
non-antialiased commands share the same renderer-owned workspace; the
standalone `DisplayList::replay` API remains self-contained and allocates a
temporary workspace for one replay. The renderer's memory accounting,
`release`, and `trim_to_budget` include both workspace families.


The CPU benchmark now includes a repeated-frame display-list stroke workload
using `DisplayListRenderer`. It records one regular and one antialiased
vector stroke, then measures replay with the renderer-owned persistent stroke
workspace. Compare this with the direct stroke and explicit `StrokeScratch`
benchmarks to separate command dispatch overhead from the benefit of retaining
geometry buffers across frames.


## Prepared geometry in display lists

Filled path commands now flatten and transform their geometry once while the
display list is recorded. The command stores the prepared `FlattenedPath`
instead of retaining the source path, transform and flattening tolerance for
repeated work during every replay. All four fill modes (even-odd, non-zero,
and their antialiased variants) consume the same prepared geometry through
internal rasterization entry points. The standalone public path APIs still
perform flattening and transformation as before. This moves deterministic
geometry preparation out of the frame replay hot path and avoids repeating
curve subdivision and point transformation for static display lists.


The benchmark suite also measures repeated replay of a curved, transformed,
antialiased display-list fill. The source path is flattened and transformed
when recorded, outside the measured frame loop; the timed replay therefore
measures the prepared-geometry path rather than charging curve subdivision
and affine point transformation on every frame.


Display lists expose `prepared_path_geometry_bytes()` to report the retained
flattened geometry storage owned by fill, path-stroke, and rectangle-outline
commands separately from the command/state-vector allocation reported by
`allocated_bytes()`. The count includes each geometry buffer's allocated
capacity. Clearing a list drops its prepared geometry while retaining the
reusable command-buffer capacity, so applications can account for the memory
tradeoff of recording static vector scenes.


## Prepared vector strokes in display lists

Display-list stroke commands now flatten curves and apply the captured affine
transform once during recording. Both regular and antialiased stroke replay
consume the retained framebuffer-space `FlattenedPath` directly; replay no
longer repeats curve subdivision or point transformation. Rectangle outlines
use the same prepared-geometry route. The public path-stroke APIs remain
available and preserve their scratch-backed preparation behavior.

`prepared_path_geometry_bytes()` now includes fill, stroke-path, and
rectangle-outline geometry, so the retained-memory figure covers all prepared
vector commands rather than fills alone. The repeated-frame CPU benchmark
already exercises regular and antialiased display-list strokes through the
persistent renderer workspace.
