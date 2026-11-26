# Strict LDS phases on Wave32 hardware

`VICTORY_LDS_PHASES=1` enables an experimental retry for a compute shader that
still requires Wave64 on a Wave32 host. It is off by default. It keeps one
64-thread console wave as a 64-thread group containing two host waves, with
collective scalar mask branches and a group barrier before each LDS access.

The accepted class is deliberately narrow: exactly 64 threads, native LDS,
one end and no indirect exits. Mask bit counts, first-set-bit, low/high prefixes
and first-active-lane reads use the existing exact cross-half exchange. Other
lane reads and writes, permutes, DPP and mask-as-data consumers remain excluded,
as do opaque and fixed lane masks. Every original
mask branch is widened, including a condition simplified to a constant by its
original incoming path. The final structure must contain no remaining
lane-dependent branch. Every emitted barrier must be outside lane guards,
in a block the whole group reaches; loop entry, exits and iteration limits
are audited too. A failed audit returns to the existing retry paths.

Saved mask ballots are evaluated at their producer, including a completed
all-lane producer tail when the value is consumed later. The barrier audit
records that producer block for the loop audit. A plain loop latch is accepted
only with a dominating header, collective header and latch, a single latch and
a source exit edge. The loop-limit return must therefore leave both host halves
together; arbitrary or closed plain backedges remain refused.

Scalar `s_waitcnt*` instructions conservatively finish memory requests for the
whole source wave with an all-memory group fence. Partial and LGKM waits are
strengthened explicitly, not treated as native UAV-completion operations.
These additional barrier points undergo the same convergence and loop audits.
Programs observing clocks or asynchronous hardware registers are excluded.

This switch does not make every shader exact or replace those existing paths.
`VICTORY_WAVE_WHY=1` distinguishes `strict LDS phases` acceptance from a legacy
retry. The translator fingerprint automatically includes the new source and
switch, so an old cache entry cannot stand in for this experiment.

Native LDS bounds remain the program's declared allocation even when the
translator adds exchange storage: out-of-range reads return zero and writes
are dropped. This part adapts canonical commit
`eabb6b850f65682dad999388d0a12bde2fd515d5` by **wwwwwwww3333333**,
with original coauthor **Claude Opus 5.5**.

## Checks on 26 November 2026

- CPU shader regressions: **183 passed, 4 ignored**. GPU test modules and the
  three inline GPU tests were excluded explicitly; no default adapter was used.
- A previously rejected real compute container, supplied outside Git, produces
  WaveSize32 and passes Windows SDK DXIL validation. The public emitter selects
  the same audited result with the switch enabled. This is an offline check.
- Manual synthetic checks on the explicitly selected **NVIDIA GeForce RTX 5070
  Laptop GPU**, software=false, wave range32..32: **72 completed readbacks**.
  They cover object-specific allocation across unequal loop lengths, native LDS
  bounds with added scratch, direct and forwarded collective exits, a repeated
  branch whose original condition looked constant, and raw VCC bits outside EXEC.
  The diagnostic test uses a loop limit of16 and tiny offscreen buffers.
- The installed WARP reports wave range4..4 and refuses the Wave64 reference
  pipeline. This is not a WARP pass. The D3D12 debug layer is unavailable.

Lanes contending at one LDS atomic address are ordered by the host; exact lane
ordering is not established. Synthetic and offline checks do not establish
correct gameplay, improved faces, stable frame rates, or game-level GPU safety.
The separate experimental launcher must be identified by its executable hash
and retained logs for the first real run.

## First native trial

The first native trial of this experiment reached 3D and created/bound the
previously refused compute pipeline, then failed with device-removed reason
`0x887A0001`. The experimental launcher is quarantined. The trial does not
establish improved rendering or frame rate; this path remains off by default.

The first error was noticed while heap descriptors were made, between GPU
submissions. Its preceding batch includes an oversized constant-buffer view;
the old creation path had no 64KiB size guard. This is a separate concrete
validation defect, not proof of the exact failing call or its relationship to
phase lowering. Runtime records remain outside Git.

The follow-up CBV validation checks size, address alignment and overflow in
both mirrored heaps and fresh tables; invalid entries become null CBVs before
any title-range allocation or copy. CPU boundary checks pass. An explicit WARP
test creates valid boundary views and rejected/null views with the device still
alive; it submits no GPU commands. That check does not identify the native
trial's exact offending API call.

The source-wait follow-up preserves wait instructions in a separate IR build;
ordinary translation keeps its previous scheduling behavior. Its regression
first failed because the ordinary IR had removed all three synthetic waits.
With retention, SDK disassembly confirms three mode11 barriers at those points.
The updated checks pass: 184 CPU tests, offline real-container validation with
26 retained waits, and 80 explicit RTX readbacks including eight cross-half UAV
store/wait/read checks.

## Follow-up native trials

The build with CBV validation and retained source waits completed a bounded
native trial with over 90 seconds of qualified 3D frames on the explicit RTX
adapter without a logged device error. This is limited stability evidence;
it does not establish the cause of the first failure or general GPU safety.
The experimental launcher now selects this build by hash, with phase lowering
enabled, separate saves and a guard against a second concurrent instance.

With the texture staging budget set to20 ms and DRED disabled, complete warm
scene windows produced 182 frames in73.1 seconds (2.49 FPS). Frame-time median
was195 ms and p95 was1089 ms. Texture staging median was50.1 ms and p95 was68.7 ms;
the budget is not a hard frame-time limit. These settings and cache/scene state
changed together, so this is not an isolated performance comparison. Faces and
lighting remain visibly incorrect. Correct, controllable 30 FPS gameplay is
still unverified.

A separate20-second recording-thread profile found substantial descriptor-lock
and physical-page mapping waits. Times summed over recording threads are not
frame time saved by removing a wait. Its sampler also paused threads for up to
44 ms, so the profiled window is not a frame-rate benchmark.

## Mask and loop extension checks

The extended candidate passes190 selected CPU regressions with5 manual checks
ignored. GPU modules, three inline GPU tests and the rectangle test module were
excluded explicitly. A second previously rejected external compute container
now validates as WaveSize32 with21 retained waits, and the public emitter selects
the same strict candidate. This is an offline compatibility result.

On the named RTX5070 adapter,224 full-buffer synthetic readbacks pass:96 cover
allocation, native LDS bounds, collective exits, source memory waits and plain
loop completion/collective loop-limit return;128 cover mask counts, first-set-bit,
prefixes and atomic-return broadcast in64x1 and8x8 groups. The latter include an
upper-half-only EXEC set, a saved mask consumed after EXEC changes, its complement
and a joined mask. Opaque/fixed masks and multiple-source-wave groups still refuse.
The extended build then created and bound that second compute pipeline in the
game, and completed90.12 seconds of qualified 3D without a logged device error.
Complete warm windows produced173 frames in72.5 seconds (2.39 FPS), with a266 ms
median and1255 ms p95. The captured cutscene still has black faces. Different
cache and scene state prevent an isolated comparison with the previous trial.
This extends bounded compatibility evidence; it does not establish correct,
controllable gameplay or a frame-rate improvement.
