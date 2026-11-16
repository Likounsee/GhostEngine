# GhostEngine — Performance targets

## CPU software renderer

GhostEngine's CPU renderer is a first-class backend, not a temporary fallback. Its performance targets must be realistic on small systems and must not assume a dedicated graphics processor.

- **Baseline target:** each representative raster operation should aim to complete within **16.67 ms** on the low-end reference machine, equivalent to 60 operations/frames per second when measured in isolation.
- **Scene budget:** a real frame contains multiple operations. The total CPU render workload, including composition and presentation overhead, must fit within the frame budget; individual benchmark rows are diagnostic and must not each be budgeted the entire frame simultaneously.
- **Quality:** performance work must preserve documented sampling, alpha-compositing, clipping and transform behavior. A faster result is not acceptable if it introduces visible artifacts or changes output unexpectedly.
- **Scalability:** benchmark single-threaded and parallel paths separately. Parallel execution is an optimization, not a requirement for small workloads where thread overhead can dominate.
- **Memory:** reusable scratch buffers and caches should have explicit bounds and predictable invalidation behavior.

The current CPU benchmark uses a 1920x1080 destination and includes both small UI-like work and large raster workloads. Its 16.67 ms indicator is a per-operation diagnostic threshold, not a universal promise for every machine or every scene. Results must always be interpreted alongside the CPU model, resolution, build mode and benchmark protocol.

## GPU backend (future work)

A future GPU backend should target substantially lower rendering latency and higher throughput where supported by the hardware. Its targets must be defined separately after selecting an API and measuring real devices. Do not compare CPU and GPU numbers without accounting for upload, synchronization, readback, presentation and workload differences.

GPU acceleration remains deferred until the CPU renderer, public resource model and backend boundaries are stable. The CPU renderer must remain fully usable and must continue to serve as a correctness reference.

## Benchmark interpretation

- Compare runs made with the same resolution, workload, build mode and iteration count.
- Use repeated runs; small CPU systems are sensitive to background load and thermal/power limits.
- Track regressions as well as improvements. A result below 16.67 ms is within the isolated-operation target, not proof that a complete application scene sustains 60 FPS.
- Keep setup costs (for example mipmap construction) separate from steady-state frame timings and report them explicitly.
