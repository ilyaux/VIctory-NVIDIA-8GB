# Strict LDS phases on Wave32 hardware

`VICTORY_LDS_PHASES=1` enables an experimental retry for a compute shader that
still requires Wave64 on a Wave32 host. It is off by default. It keeps one
64-thread console wave as a 64-thread group containing two host waves, with
collective scalar mask branches and a group barrier before each LDS access.

The accepted class is deliberately narrow: exactly 64 threads, native LDS,
one end, no indirect exits, and no original lane reads, permutes, DPP, mask
values used as numbers, or other cross-lane instructions. Every original
mask branch is widened, including a condition simplified to a constant by its
original incoming path. The final structure must contain no remaining
lane-dependent branch. Every emitted barrier must be outside lane guards,
in a block the whole group reaches; loop entry, exits and iteration limits
are audited too. A failed audit returns to the existing retry paths.

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
