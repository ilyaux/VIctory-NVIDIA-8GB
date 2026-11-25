# RTX 5070 Laptop performance investigation

Target: stable 30 FPS in real gameplay on an RTX 5070 Laptop 8 GB,
Ryzen AI 7 350 and 32 GB RAM. Menu FPS does not satisfy the target.

## 2026-11-26: first native baseline

Build: `1e34f02`, Windows 11, driver 621.24, diagnostics profile, Series X.
Levels on demand enabled; mip reclamation disabled. A 100 Hz sampled profiler
and the built-in frame logger were enabled. Test saves were copied from the
user's data into an isolated directory. Game files, captures, caches and raw
logs remain outside Git.

- Startup/menu: around 30 FPS, median frame time approximately 33 ms.
- Transition into the intro and credits: observed frame-rate lines between
  0.3 and 1.8 FPS, with hundreds of pipelines still pending. These are warm-up
  observations, not a steady-state gameplay benchmark.
- Preparation on the CPU took hundreds of milliseconds and sometimes over a
  second. Texture refreshes spent substantial time waiting for title memory
  to stop being remapped. The frame logger showed substantial GPU idle time.
- The baseline ended with GPU device removal `0x887A0006`. There were no DRED
  breadcrumbs in that run. It is not a stable baseline.
- The external environment had `RUST_LOG=warn`. Benchmark launches now set
  `VICTORY_LOG=info` explicitly so the required timings are retained.

The next run tests the existing fast switch set documented in
[the inherited README](../README-fxceless.md), with DRED enabled. Its pipeline
cache is warmer, so it cannot establish the isolated speedup of that switch
set. A warm control run and comparable scenes are required.

## Measurement rules

1. Record the commit, executable hash, console profile, all overrides, driver
   and selected D3D12 adapter. Host startup now logs the adapter's name,
   vendor/device IDs and dedicated memory.
2. Retain frame times, draw and dropped-draw counts, pending pipelines,
   resource preparation, GPU activity, memory budgets and faults. Report
   loading, compilation and gameplay separately.
3. Compare the same scene and camera after warming the relevant pipelines;
   repeat the control when cache state changes. A preset comparison does not
   attribute a result to its individual switches.
4. Check the picture, streaming, input, device removal and memory growth.
   Skipping draws, removing required passes or reporting menu FPS as gameplay
   is not a performance result.
5. Keep native and WARP results separate. Claim completion only after repeated
   sustained gameplay at the requested frame rate, including frame-time tails
   and scene transitions.

No stable 30 FPS gameplay result has been established.
