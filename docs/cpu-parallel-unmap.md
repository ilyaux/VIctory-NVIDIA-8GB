# Whole-view unmap experiment

`VICTORY_UNMAP_PARALLEL=4` opts into a CPU memory experiment. It is off by
default. It applies only to 4 through 4096 distinct, nonoverlapping tracked
views, wholly inside the requested unmap range. Addresses and view sizes are
page aligned. Gaps between views are permitted; partial views and their tails
keep the original path. No page mappings are coalesced or created in workers.

The calling thread holds the existing physical-table lock and remapping write
guard. It removes each view and its reverse entries and issues content
notifications in the original order. Section references remain alive. At most
four scoped workers perform native unmaps of those independent old views; all
workers join before a later map, return or lock release.

Kernel-copy texture readers can bypass the remapping guard. They may re-arm
write tracking and read an old view after the initial notification. A second
serial notification after all native calls invalidates that observation,
including when its cached bytes are committed later. This prevents a durable
clean-cache skip; it does not make existing kernel snapshots linearizable or
remove existing partial-texture refresh delays. Arbitrary guest CPU accesses
are not protected by this guard; callers still own synchronization of the
range they ask to unmap.

Full old views are discarded, including private COW data, as on the original
path. Unrequested aliases and pages retain their identity. The restriction
avoids the surviving-tail COW issue of changing view granularity.

The existing caller ignores native unmap failure. This experiment preserves
that ABI behavior, counts observed failures and logs an error for the bounded
controller to stop. It makes no recovery or transaction guarantee after a
native failure. Thread creation failure falls back to that same chunk
sequentially. A panicking chunk is reported separately; its unknown remainder
is not retried. Such a run is invalid for performance or correctness claims.

The cumulative parallel-unmap line includes requests, views, batch wall time,
native failures, spawn fallbacks and panicked chunks. Native API phase times
sum worker wall time and may overlap; compare batch wall time separately.
They are not extra elapsed time to add to frame time or lock waits.

Native measurements established the complete-view request class and expensive
unmaps in the tested scene. A separate cold 32-MiB Python/ctypes fixture did
not reproduce that latency or improve with four workers. The proposed path
therefore requires CPU semantic checks and a same-binary native control and
candidate before any benefit or suitable default can be claimed. Stable
correct 30 FPS remains a separate game-level requirement.

Related work: the parallel VM approach in
[Belobug's fork](https://github.com/Belobug/VIctory-Belobug-AMD/blob/81ebb7bdc9b7ca6ec2f3169973f50be77c7c803c/crates/victory-hle/src/modules/xmem.rs)
was a reference while investigating the measured native API costs.
