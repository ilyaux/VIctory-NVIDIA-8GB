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

The first exact-merge native attempt (run11) was stopped after about 58
seconds when the user reported a black picture with audible title audio.
Both run10 and run11 were still in their low-draw loading phase at that
time, with zero changed runs uploaded under flights. Run10 first dispatched
the upscaler at about 88 seconds. The changed gap setting was therefore not
exercised in the observed interval; neither a visual improvement nor a
gap-induced regression was established. No device-removal error was logged.

The host previously ignored the DXGI `Present` result, so missing error logs
could not establish successful presentation. It now logs nonzero status
changes and recovery, and reports failed presents as not shown. The queue's
periodic counter is labelled as presentation requests. This is diagnostic
coverage, not a fix for the black picture; a targeted source-frame readback
is still needed to distinguish empty rendering from a presentation failure.

Run12 made that distinction with internal source-frame readback on the
explicit RTX adapter. Of 185 saved frames, the early frames were entirely
black before DXGI; four later world frames contained an image. No nonzero
DXGI presentation status was logged. The first world image was frame 2848.
Black faces, missing geometry and exposure defects remained. Capture waits
make this a correctness observation, not a comparable FPS result.

The command trace records video-plane uploads, conversion into an RGB
texture and composition into a 3840x2160 UI target, while this fork presents
only the first plane's 1920x1080 texture. Canonical
[0736f8b](https://github.com/Marvisak/VIctory/commit/0736f8b375c3ae95fca0a21af6aa0b6cbc1251cf)
by Calvix2k addresses missing Series S UI with ordered multi-plane
composition. The adapted change is in `362c44b`, retaining its original
author and co-author credits.

The adapted compositor preserves the single-plane ahead path; multiple
planes complete synchronously so their descriptor heaps can be reused
safely. The console plane stride and field offsets passed a CPU-only test.
Three manual tests passed first on explicit WARP and then on the exact RTX
5070: spatial 2x2-to-8x8 scaling, three-plane premultiplied blending and
order, repeated source state transitions, retained source identities and
failure fallback followed by retry, and an unsampleable depth-only source
that falls back before SRV creation and recovers with valid input. A CPU
regression checks typed formats, typeless mapping and rejected source
descriptions. Tests inspect completed pixel readback
before flip-discard presentation. They are ignored by default and require
both an explicit device selection and `VICTORY_PRESENT_GPU_TESTS=1`; the
actual device identity is checked before recording graphics commands.
Normal and diagnostics compilation passed.

Native run13 recorded two actual planes: a 1920x1080 scene and a 3840x2160
UI target. Composed backbuffer readbacks at frames 256 and 512 contained
the intro image; the early source readbacks in run12 had been entirely
black. No nonzero DXGI present status or unsupported-source fallback was
logged. This verifies rendered intro content, rather than physical screen
observation, complete 3D correctness or an FPS improvement. Capture and
synchronous composition make this unsuitable as a performance comparison.

A separate placed-UAV adaptation was rejected during static review before
any GPU execution. Replacing a filled read-only backing with a new writable
backing can lose untouched texels and mip levels on a partial write. Full
overwrite tests do not cover that contract. Default views also need a valid
typed format when storage becomes typeless. That candidate is outside the
working branch and is not treated as a rendering fix.

## Narrow CPU profile after the compositor change

Run14 used the same `362c44b` binary, explicit RTX adapter, Series S,
host FidelityFX provider 2.3.4, exact upload gaps and isolated first
checkpoint. Capture and DRED were off. A 25 Hz sampler followed two
submission-role threads for 20 seconds after 180 seconds of warm-up.
The actual selected threads included `victory-submit`. It observed 72
presentations, averaging 278.2 ms in this changing streaming scene. This
is a diagnostic interval, not a comparable steady gameplay benchmark.

Median thread suspension was 41 microseconds; p99 was 5243 microseconds
and the maximum was 21837 microseconds. The sampler used 2.8% of one CPU
core. Instrumentation overhead therefore remains a limitation.

The submit worker spent approximately 176 ms per frame in its condition
variable wait. The title thread spent about 155 ms waiting for a title
event and 91 ms beneath `release_held_now`. These times come from different
threads and cannot be added or assumed to be removable stalls. The aggregate
profile does not identify the specific mutex or establish a missed wake-up.

A separate measured candidate is texture destruction: about 20 ms per
frame beneath `retire<HostTexture>`, including driver allocation and GPU
virtual-address destruction. Source review confirms idle texture sweeping
runs before the next submission is counted as started, so retirement drops
objects immediately when no submission remains in flight. Disabling
`VICTORY_TEXTURE_SWEEP` is only a bounded diagnostic comparison: it retains
idle texture memory and is unsuitable as an unbounded 8 GB preset. Generic
retirement also runs mip-publication callbacks, so moving all retired
objects to a background thread would change correctness contracts.

The first bounded control (run15) reached the free-RAM guard before its
planned sampler interval. Shorter probes ran the sampler for 15 seconds
after 60 seconds: run17 with texture sweeping and run18 without it. The
control had not reached its first host upscaling dispatch when stopped;
the second run reached it at about 60 seconds. Their different rendering
phases prevent a causal FPS comparison. The disabled run had no texture
sweep calls in its sampled stacks, confirming the switch took effect,
but this does not demonstrate a performance gain. User launch settings
retain texture sweeping. Future comparisons must match scene progress,
rather than relying on fixed-time Enter presses and warm-up alone.

## Wave64 investigation: the first LDS refinement remains insufficient

An unmerged candidate refines the mixed-LDS rejection for exactly 64
threads. It requires singleton invocation sets for overlapping accesses,
or disjoint byte ranges; integer atomic additions are exempted only when
their returned values are unused. A preliminary logical-half rule was
rejected because logical thread coordinates do not prove physical host
wave membership. The corrected singleton rule passed 11 CPU regressions
and independent source reviews, including Claude Code Opus 5.5.

The first real failing compute-program comparison was CPU-only, with
local-wave and forced-wide modes disabled. Both control and candidate
emitted DXIL and passed the SDK validator, but their outputs were identical
and still declared WaveSize64. Validator success alone does not establish
RTX compatibility. No candidate GPU or game execution followed.

Tracing found missing known-bit handling for `v_lshl_add_u32`. Reusing
the existing shift/add rules now recognizes the singleton finalizer; a
new original synthetic regression covers a group-ID shift combined with
a local thread ID. All 12 CPU regressions passed after removing temporary
case-inspection instrumentation. The real program still retains Wave64:
an atomic return affects a clipped allocation count, another atomic's
increment and output records. The candidate remains outside this branch.

An original synthetic counterexample explains the remaining restriction.
With a shared limit of one, invocation 0 requests amounts `[0, 1]` over
two iterations and invocation 32 requests `[1, 0]`. Preserving the original
iteration phases grants the budget to invocation 32. Independent half-wave
progress can grant it to invocation 0 instead, despite an equal total.
Observed returns therefore need an ordering contract; treating them as
unused counters would change work allocation. The next investigation is
uniform loop phases with barriers outside divergent guards.

Placed-UAV diagnostics also needed a narrower first sample: run19's first
warning came from a null-address placeholder. Such descriptors now retain
the same null fallback without consuming the one-time warning. Run20 used
the explicit RTX and provider 2.3.4, stopped after 35 seconds, and logged no
nonnull unsupported placed view. That early interval cannot rule out the
path during later gameplay or attribute black faces to it.
