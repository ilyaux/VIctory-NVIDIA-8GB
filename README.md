# VIctory for NVIDIA 8 GB

A personal development fork of
[fxceless/victory-fork](https://github.com/fxceless/victory-fork), focused on
AMD CPUs and NVIDIA RTX 50-series GPUs, tested with 8 GB of VRAM. The first
priority is memory use and stability, followed by frame time in measured scenes.

**No game files are included.** Supply your own copy and keep the game, caches,
logs and saves outside this repository. See [the asset policy](docs/asset-policy.md).

## Current status (2026-11-26)

Native runs have started on this machine. Startup/menu runs at approximately
30 FPS; the first intro runs fell to roughly 0.3-3 FPS with pipelines still
compiling. The initial baseline also ended with GPU device removal
`0x887A0006`. Stable 30 FPS gameplay has not been established. See the
[performance investigation](docs/performance-nvidia-8gb.md) for evidence and limits.
The automated GPU regressions below were run on WARP, the software adapter.

| Test platform | Public hardware summary |
| --- | --- |
| GPU | NVIDIA RTX 50-series |
| CPU | AMD |
| Video memory | 8 GB VRAM |

This starts from `fxceless/victory-fork` at `9c184b3`, whose author reports
testing on an RTX 4070. The inherited history and documentation are kept in
[README-fxceless.md](README-fxceless.md). Its gameplay numbers describe the
upstream author's machine.

## Changes carried here

- Deferred release of tiled-texture backing from **LeonardoS11**,
  [PR #795](https://github.com/fxceless/victory-fork/pull/795), original commit
  `32772d93ea1ec529a8508d08e980c3867c2701ff`. Original commit metadata is preserved.
- Experimental reclamation of unused mip backing, retained-reader and upload-fence
  regressions, and reuse of free pool tiles without heap growth. This work is
  available for discussion in [Draft PR #1153](https://github.com/fxceless/victory-fork/pull/1153).

Reclamation remains opt-in: `VICTORY_LEVELS_ON_DEMAND=1` together with
`VICTORY_LEVELS_RECLAIM=1`. Old resources stay alive for recorded/in-flight
readers, and indexed views wait for the replacement upload's completion fence.
GPU-written or claimed levels block reclamation; packed tails stay together.

### Validation of `b472041`

| Check | Result |
| --- | --- |
| WARP diagnostic GPU suite | 202 passed, 6 ignored, 2 explicitly excluded |
| Tile regressions, including ignored tests | 6 passed |
| Diagnostic application check | Passed |
| Repository asset policy | Passed (185 files before this documentation update) |

Both excluded tests also fail on the unmodified base `9c184b3`:
`depth_bounds_pipeline_loads_and_filters_draws` and
`warp_budget_saturates_without_wraparound_and_preserves_draw_fields`.
The full workspace test attempt fails at the latter. Both strict workspace
Clippy attempts fail in unchanged shader code; the local Clippy version also
differs from the installed rustc/cargo toolchain. Full CI validation remains open.

For synthetic BC7 and BC3 textures, assigned backing fell from 1856 KiB to
512 KiB per texture while old readers and subsequent fine-mip uploads remained
valid. This is backing accounting, not a measurement of total process VRAM.
Native GPU mapping, including packed tails, still needs validation; the local
native tile test requires the D3D12 debug layer, which is not installed.

## Build

Use Windows x64, the pinned Rust toolchain and MSVC build tools.

```powershell
git clone https://github.com/ilyaux/VIctory-NVIDIA-8GB.git
cd VIctory-NVIDIA-8GB
Copy-Item victory.example.toml victory.toml
# Set your paths in victory.toml, outside this repository.
cargo build --locked --profile diag -p victory --features diagnostics
```

## First hardware comparisons

For each run, confirm that the logged D3D12 device is the intended NVIDIA GPU.
Start with the existing adaptive memory budgeting; an explicit
`VICTORY_TEXTURE_BUDGET_MB` limits texture-copy accounting and does not cap total
VRAM use. No fixed 8 GB tuning preset has been established yet.

Use the same build, scene, console profile and other settings for both runs.
Run the baseline with `VICTORY_LEVELS_ON_DEMAND=1` and
`VICTORY_LEVELS_RECLAIM=0`, then repeat with reclamation set to `1`.
WARP must be unset for either hardware run.

The inherited launch command is:

```powershell
.\target\diag\victory.exe run '<path to your game executable>' --execute --console series-x
```

Record frame times, local/shared GPU memory, process commit and texture residency.
Check missing textures, device removal, old readers and detail returning after
streaming. Change one setting at a time and retain the baseline results.

## Credits and license

- [Marvisak/VIctory](https://github.com/Marvisak/VIctory) and its contributors:
  original compatibility layer.
- [fxceless/victory-fork](https://github.com/fxceless/victory-fork) and its
  contributors: the base of this fork; full inherited credits are preserved in
  [README-fxceless.md](README-fxceless.md).
- LeonardoS11: backing-lifetime fix carried as a separate commit.
- This fork's reclamation changes, synthetic tests and documentation were
  developed with Codex.

GPL-3.0-or-later; see [LICENSE](LICENSE) and the exception documented in the
[asset policy](docs/asset-policy.md).
