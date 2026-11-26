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

Branch `perf/rtx5070-card-hash` carries Martin Hartfelder's
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

This supports reduced time in the layer's hash work on the RTX 5070. The scene
progression and workloads differed, and the second run had warmer pipelines;
the usual-frame logger excludes long frames. The table therefore does not
establish an isolated full-frame speedup or sustained gameplay performance.
Rendering defects remained in both runs. Neither short run logged device
removal; both were deliberately stopped. The 30 FPS goal remains unmet.

## Host FidelityFX and the remaining picture defects

Branch `perf/rtx5070-fidelityfx-host` selectively adapts the optional host
FidelityFX bridge from Marvis's
[0b35b7f](https://github.com/Marvisak/VIctory/commit/0b35b7fe7d728c9a9c205cb3719a0f11744321ce),
with original co-author Claude Fable 5.2. No SDK DLL is included. The bridge
requires `VICTORY_FIDELITYFX_DIR`, an explicitly selected hardware adapter and
`VICTORY_FIDELITYFX_PROVIDER`. Unavailable or ambiguous selections fail closed.
Without an SDK directory the original title DLL path remains in use.

Use `VICTORY_D3D12_ADAPTER=NVIDIA GeForce RTX 5070 Laptop GPU` on this machine.
A test executable does not inherit the game's Windows GPU preference: the
first manual integration test selected the AMD integrated adapter, failed
with zero output, and coincided with a user-reported display blackout and DWM
restart. That failed test is not validation. The fixture now requires an
explicit adapter, checks its identity before loading the SDK, and waits for
the asynchronous queue before readback and context destruction.

With the official SDK 2.4.0 and provider `2.3.4`, the full flat-colour
1280x720-to-1920x1080 integration test passed on WARP and the explicitly
selected RTX 5070. All output RGB pixels were checked, over two dispatches.
Four bridge unit tests and the metadata-only adapter rejection test passed.
The integration test is manual and ignored by default. Providers 3.1.5 and
3.1.6 faulted inside WARP's compute compiler and were not retried on hardware.

Native run10 used Series S, the first isolated checkpoint and host provider
2.3.4. Actual context creation and upscaling dispatch were logged, bypassing
the unsupported Xbox upscaler programs. Gameplay still had black faces and
unstable objects; occasional 8 FPS does not establish a speedup. In the last
two minutes, 24 usual-frame samples had a median frame time of 119 ms, with
75 ms preparation, 44 ms copying and 35 ms heap work. These counters overlap
and must not be added together. The per-list GPU meter also needs independent
validation before being used to infer a hardware FPS ceiling. The run was
closed normally after about eleven minutes.

### Correctness before more aggressive settings

Run10 inherited `VICTORY_MERGE_GAP=64` from the experimental fast preset. This
joins uploads across unchanged CPU bytes and can overwrite shader stores in
those gaps with old CPU values. The exact default is `VICTORY_MERGE_GAP=0`.
The CPU-only regression
`shadow::merge_tests::exact_merge_preserves_gpu_stores_between_title_changes`
covers gaps of 1, 64 and 65 bytes, with GPU stores inside and outside them.
Exact uploads preserve every store; the approximate mode is a positive
control demonstrating loss within its 64-byte threshold. All six merge tests
passed. This proves the mechanism, not its attribution to a visible object.
The next native comparison changes only the gap setting to zero.

Two separate unsupported paths remain: placed texture UAV descriptors bind
null views, and seven rejected compute programs still emit wave64 on this
wave32 adapter. Canonical
[61bfc77](https://github.com/Marvisak/VIctory/commit/61bfc773d3296252918572921237075d8a748649)
addresses placed UAVs but depends on shared host-texture ownership missing
from this fork. It requires an adapted port and view-order/lifetime tests.
Forcing wave32 does not preserve the lane and shared-memory semantics of the
failed shaders; forcing wave64 has a documented device-loss history. Neither
switch is treated as a fix. Attribution of these paths to black faces is
still open. Correct picture and stable 30 FPS remain unverified.
