# Reusing identical physical aliases

`VICTORY_ALIAS_REUSE=1` enables an experimental CPU path in `xmem::map_alias`.
It is off by default. It tries to retain an existing mapping instead of
unmapping and mapping the same section pages again. This does not change shader
translation, rendering resolution, draw counts or the native memory layout.

The candidate must cover complete, individually mapped 64 KiB destination views.
Source and destination ranges must be disjoint, aligned and non-overflowing.
Each destination must name the same owned section and page index that the
source lookup would map. Partial larger views, missing pages and mismatched
backing take the existing path. Every candidate miss or API failure also falls
back to that entire path; it does not return a new failure or preserve a stale
destination when the original operation would unmap it.

Matching bookkeeping is insufficient after copy-on-write. A private copy can
still appear as mapped memory, and changing its protection back to read/write
does not restore the section's bytes. After reopening the views, every host
page must report both valid and shared through `QueryWorkingSetEx`; private,
nonresident and failed queries decline reuse. No pages are touched or pinned
to make this proof succeed. The host page size comes from `GetSystemInfo`.
See [VirtualQuery](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualquery)
and [working-set flags](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-psapi_working_set_ex_block).

The ordinary remapping exclusion remains held. Before reopening, the runtime
notifies texture snapshots, descriptor tracking and in-place views as it does
before an unmap. A narrow watch helper holds the guard mutex while changing a
whole view to read/write and clearing its protection bookkeeping. A later guard
can re-arm without being forgotten. Queries run after all these resets; ordinary
writes can no longer create COW through a previously copy-on-write protection.
Successful views are registered again without a second blind guard-state clear.
Physical reverse mappings and section references stay unchanged.

This assumes runtime ownership of the tracked mappings, as the existing mapper
does. Unsynchronized external virtual-memory mutations are outside that contract.
The frame log reports retained and examined alias pages so a native experiment
can establish whether this path is used. Proof/reset costs and low residency
can outweigh saved mapping calls; compatibility tests alone establish no FPS gain.

## CPU checks

The 16 selected xmem tests pass with reuse both enabled and disabled. Four new
Windows fixtures cover resident aliases, private COW in the last host page,
read-only reset, unchanged reverse entries, partial views, overlap, source
failure, checked-end overflow and an uncommitted residency-query refusal.

The default watch selection passes 15 tests with one manual event-log-writing
test ignored. A concurrent guard/reset test with 66 aliases also passes alone
with `VICTORY_VIEW_GUARD=0`. Running the entire old watch suite with that switch
disabled fails four tests that expect cross-view guarding; that run is not a
pass. Native usage, rendering and frame-rate comparisons remain required.
