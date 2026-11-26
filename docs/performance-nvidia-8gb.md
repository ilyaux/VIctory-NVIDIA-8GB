# NVIDIA 8 GB performance investigation

Target: stable 30 FPS in real gameplay on an AMD CPU and an NVIDIA RTX
50-series GPU with 8 GB VRAM. Menu FPS does not satisfy the target.

## 2026-11-26: first native baseline

Build: `1e34f02`, diagnostics profile, Series X.
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

## Warm runs and the Series S upscaler

The warm Series X run loaded 1662 cached pipelines in about seven seconds.
Intro scenes reached 4.6-6.8 FPS with no pipelines pending, but a later courtyard
scene fell to around 0.3 FPS with 4906-7704 draws per frame. The reported memory
use exceeded the 7123 MiB DXGI budget (7265-7361 MiB in use, plus substantial
shared memory). Cold compilation therefore does not explain the whole problem.

Series S reduced reported GPU memory use to about 4780 MiB. With the diagnostic
sampler and DRED removed, some later scenes reached 7-8 FPS. These are different
scenes, not a controlled speedup measurement, and the picture was invalid:
20 compute shaders failed to translate. This result is not successful gameplay.

### Experimental ACCC/FSPX support

Branch `perf/fspx-upscaler` backports selected translator work from
[Marvisak/VIctory commit 0b35b7f](https://github.com/Marvisak/VIctory/commit/0b35b7fe7d728c9a9c205cb3719a0f11744321ce),
authored by Marvis, with original co-author Claude Fable 5.2. It includes the
FSPX compute metadata parser, raw-descriptor tracing, signed packed dot products,
packed half-precision multiply-accumulate, SCC operands and constant sampler
words. The runtime path requires `VICTORY_FSPX=1`; it remains disabled by default.

The added regression fixtures are synthetic. They cover malformed metadata,
resource bounds, ACCC-to-DXIL translation and arithmetic on the hardware GPU and
WARP. Windows SDK `dxil.dll` validated the synthetic ACCC output. The final
full shader suite passed 224 tests, with three ignored. The diagnostics build
and repository asset-policy check also passed.

An offline scan of the locally supplied upscaler DLL found 29 compute programs.
All 29 now parse, structure and track resources; 23 emit DXIL and pass validation.
Six still stop at unsupported lane-mask, half-precision image-atomic or root-
descriptor operations. An offline CLI exit code of zero does not mean that
every program translated: inspect its emitted and validated counts.

The native Series S probe requested 20 ACCC programs. Matching their complete
ACCC payloads against the supplied DLL identified all 20; 15 translated and
five remained unsupported (four lane-mask cases and one half-precision image
atomic). No host pipeline refusal named those ACCC shaders. This establishes
translation coverage, not correct execution of the complete upscaler. The
picture remained invalid and this run is not an FPS improvement result.

Complete picture, controls and comparable frame times remain to be checked.
No game binaries, shader dumps, screenshots, saves or raw logs are included in
this repo.

## Opt-in parallel hashing of card copies

The parallel-hash experiment carries Martin Hartfelder's
[64-thread card hash](https://github.com/fxceless/victory-fork/commit/55572246864888974da5bf8f418a092e73d66932),
with the opt-in selection from fxceless's
[6d20eed](https://github.com/fxceless/victory-fork/commit/6d20eed6ae8afe37c52f1911b46537b0ea8bcd0d).
`VICTORY_CARD_HASH_WIDE=1` distributes each piece's hash across 64 threads;
the default retains the previous hash. The store and compare always select
the same form for a process. Both WARP regressions passed for both forms,
including changed-byte offsets in skewed spans; the diagnostics build passed.

Native runs 08 and 09 used the same binary, isolated checkpoint, Series X
profile and fast preset. Only the hash flag changed. The following are medians
of usual-frame log lines with at least 1000 draws and at least three lists of
each measured layer-work kind (20 control lines, 19 wide-hash lines):

| Metric | Previous hash | 64-thread hash |
| --- | ---: | ---: |
| Send-and-hash lists per frame | 23.9 ms | 5.1 ms |
| Compare lists per frame | 6.8 ms | 2.5 ms |
| Send-and-hash time per list | 4.10 ms | 1.00 ms |
| Compare time per list | 1.14 ms | 0.48 ms |
| Draws per usual frame | 3838 | 3501 |
| Title GPU lists per usual frame | 73.0 ms | 57.1 ms |
| Usual frame time | 175 ms | 152 ms |

This supports reduced time in the layer's hash work on the tested NVIDIA GPU. The scene
progression and workloads differed, and the second run had warmer pipelines;
the usual-frame logger excludes long frames. The table therefore does not
establish an isolated full-frame speedup or sustained gameplay performance.
Rendering defects remained in both runs. Neither short run logged device
removal; both were deliberately stopped. The 30 FPS goal remains unmet.
