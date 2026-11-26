# Repeated-page mapping experiment

`VICTORY_REPEAT_BATCH=1` enables an experimental CPU path for the queue's
repeated-source page ranges. It is off by default and independent of alias reuse.
It applies only when the destination is exactly one whole old view of at least
two pages and the known source page does not overlap that destination range.
Other requests keep the original page-at-a-time path.

The old loop replaces one page at a time. Each replacement inside a larger view
unmaps that entire view, then maps its surviving tail again. Repeating this over
N pages therefore reprocesses N(N-1)/2 tail pages. The restricted batch unmaps
the old view once, then maps each destination page to the SAME captured source
page. It does not turn a repeated-source request into consecutive source pages.
No placeholder coalescing is needed for this exact whole-view case.

Actual Windows CPU operation-count fixtures pass:

| Destination pages | Old map / unmap calls | Batch map / unmap calls | Old / batch tail pages |
|---|---|---|---|
| 4 | 7 / 4 | 4 / 1 | 6 / 0 |
| 8 | 15 / 8 | 8 / 1 | 28 / 0 |

These count production mapping functions on owned allocations, excluding setup
and teardown. They establish eliminated work for the fixture, not a game FPS gain.
Frame telemetry records actual range kinds, candidate pages, mapping attempts,
tail pages and reuse rejection reasons for native checks.

## Failure and concurrency contract

The physical and remapping locks stay held for the batch. Source identity must
remain stable for the queue operation, as for its existing source ranges. The
first legacy page replacement would already unmap the complete old view,
discarding its private COW pages. That is why the old section slice can restore
the untouched suffix in this restricted class. Separate old page views are
excluded: their later private COW data would survive an early legacy failure.

If mapping page j fails, the new prefix remains, page j stays a hole, and the
strict old suffix after j is restored from its old section/index. A failed
suffix restoration is logged and the mapper still reports failure. Recovery
cannot be guaranteed if that restoration or native cleanup itself fails; no
success or transactional guarantee is claimed for such a host failure.

The 23 selected xmem CPU tests pass with batching on and off, with one manual
operation-count test ignored; that manual test also passes separately. Fixtures
cover distinct page contents, failure at every position, real split/map/rollback
states before registration, COW in a large view, excluded separate COW views,
public callback outcomes, reverse mappings and a second restoration failure.
They do not force a real Windows commit OOM or native cleanup failure.

The runtime registers a separate repeated-page callback; other users of the
original consecutive-page mapper retain its signature and behavior. The queue
also keeps its original fallback when no repeated callback is installed.
Native eligibility, image fidelity and frame-rate effects remain unverified.
