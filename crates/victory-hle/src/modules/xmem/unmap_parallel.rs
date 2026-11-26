//! Optional native calls only; the caller retains sections and owns metadata/guards.
use super::PAGE;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("VICTORY_UNMAP_PARALLEL").is_some_and(|v| v == "4"))
}

pub(super) fn qualifies(views: &[(usize, usize)], base: usize, end: usize) -> bool {
    if !(4..=4096).contains(&views.len()) || base >= end || (base | end) & (PAGE - 1) != 0 {
        return false;
    }
    let mut previous_end = base;
    for &(at, len) in views {
        if at == 0 || len == 0 || (at | len) & (PAGE - 1) != 0 || at < previous_end {
            return false;
        }
        let Some(limit) = at.checked_add(len) else { return false; };
        if limit > end {
            return false;
        }
        previous_end = limit;
    }
    true
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Outcome {
    pub(super) failures: usize,
    pub(super) spawn_failures: usize,
    pub(super) panicked_chunks: usize,
}

impl Outcome {
    fn add(&mut self, other: Self) {
        self.failures += other.failures;
        self.spawn_failures += other.spawn_failures;
        self.panicked_chunks += other.panicked_chunks;
    }
}

fn run_chunk(views: &[(usize, usize)], unmap: &(impl Fn(usize, usize) -> bool + Sync)) -> Outcome {
    let mut result = Outcome::default();
    // A panic leaves this chunk's remaining calls unknown/unattempted, never retried.
    let caught = catch_unwind(AssertUnwindSafe(|| {
        for &(at, len) in views {
            if !unmap(at, len) {
                result.failures += 1;
            }
        }
    }));
    if let Err(payload) = caught {
        // A panic payload can itself panic on drop; never unwind through the FFI caller.
        std::mem::forget(payload);
        result.panicked_chunks = 1;
    }
    result
}

/// Only independent native unmaps belong in the callback; all joins finish here.
pub(super) fn run(views: &[(usize, usize)], unmap: impl Fn(usize, usize) -> bool + Sync) -> Outcome {
    let mut result = Outcome::default();
    if views.is_empty() {
        return result;
    }
    std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(4);
        for chunk in views.chunks(views.len().div_ceil(4)) {
            let unmap = &unmap;
            match std::thread::Builder::new().spawn_scoped(scope, move || run_chunk(chunk, unmap)) {
                Ok(worker) => workers.push(worker),
                Err(_) => {
                    result.spawn_failures += 1;
                    result.add(run_chunk(chunk, unmap));
                }
            }
        }
        for worker in workers {
            match worker.join() {
                Ok(outcome) => result.add(outcome),
                // No retry: even an unexpected worker panic may follow completed calls.
                Err(payload) => {
                    std::mem::forget(payload);
                    result.panicked_chunks += 1;
                }
            }
        }
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{alias_tests::Aliases, tests::query, *};
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_WRITECOPY, VirtualProtect};

    fn views(count: usize) -> Vec<(usize, usize)> {
        (0..count).map(|i| ((1 + 2 * i) * PAGE, PAGE)).collect()
    }

    #[test]
    fn eligibility_requires_whole_sorted_aligned_views_but_allows_gaps() {
        let v = views(4);
        assert!(qualifies(&v, PAGE, 8 * PAGE));
        assert!(qualifies(&views(4096), PAGE, 8192 * PAGE));
        assert!(!qualifies(&v[..3], PAGE, 8 * PAGE));
        assert!(!qualifies(&views(4097), PAGE, 8194 * PAGE));
        assert!(!qualifies(&v, 2 * PAGE, 8 * PAGE));
        assert!(!qualifies(&v, PAGE, 7 * PAGE));
        assert!(!qualifies(&v, PAGE + 1, 8 * PAGE));
        for changed in [(0, PAGE), (PAGE + 1, PAGE), (PAGE, 0), (PAGE, PAGE + 1)] {
            let mut bad = v.clone();
            bad[0] = changed;
            assert!(!qualifies(&bad, 0, 8 * PAGE));
        }
        for changed in [(PAGE, PAGE), (PAGE, 3 * PAGE), (5 * PAGE, PAGE)] {
            let mut bad = v.clone();
            bad[1] = changed; // duplicate, overlap, or out of order.
            assert!(!qualifies(&bad, PAGE, 8 * PAGE));
        }
        let top = usize::MAX & !(PAGE - 1);
        let mut overflow = v;
        overflow[3] = (top, PAGE);
        assert!(!qualifies(&overflow, PAGE, top));
    }

    #[test]
    fn every_view_is_called_once_with_at_most_four_active_callbacks_and_joined() {
        let calls: Vec<_> = (0..17).map(|_| AtomicUsize::new(0)).collect();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let v: Vec<_> = (0..17).map(|i| (i * PAGE, PAGE)).collect();
        let result = run(&v, |at, len| {
            assert_eq!(len, PAGE);
            let now = active.fetch_add(1, Relaxed) + 1;
            peak.fetch_max(now, Relaxed);
            calls[at / PAGE].fetch_add(1, Relaxed);
            std::thread::yield_now();
            active.fetch_sub(1, Relaxed);
            true
        });
        assert_eq!(result.failures, 0);
        assert_eq!(result.panicked_chunks, 0);
        assert!(peak.load(Relaxed) <= 4);
        assert_eq!(active.load(Relaxed), 0);
        assert!(calls.iter().all(|n| n.load(Relaxed) == 1));
        assert_eq!(run(&[], |_, _| panic!("empty callback")), Outcome::default());
    }

    #[test]
    fn false_results_and_panics_are_not_retried_and_preserve_observed_failures() {
        let calls: Vec<_> = (0..12).map(|_| AtomicUsize::new(0)).collect();
        let v: Vec<_> = (0..12).map(|i| (i * PAGE, PAGE)).collect();
        let result = run(&v, |at, _| {
            let i = at / PAGE;
            calls[i].fetch_add(1, Relaxed);
            assert_ne!(i, 1, "injected callback panic");
            i != 0 && i != 11
        });
        assert_eq!(result.failures, 2); // includes false before the first chunk's panic.
        assert_eq!(result.panicked_chunks, 1);
        assert_eq!(calls[2].load(Relaxed), 0, "remaining panicked chunk is not replayed");
        for (i, called) in calls.iter().enumerate() {
            if i != 2 { assert_eq!(called.load(Relaxed), 1); }
        }
        // This is also the spawn-error inline path's panic/failure consumer.
        let inline = run_chunk(&v[..3], &|at, _| {
            assert_ne!(at, PAGE, "injected inline panic");
            false
        });
        assert_eq!((inline.failures, inline.panicked_chunks), (1, 1));
    }

    fn fill(a: &Aliases) {
        // SAFETY: this fixture owns the committed source and all its aliases.
        unsafe { (a.source as *mut u8).write_bytes(0x31, a.pages.len() * PAGE) };
    }

    fn middle(a: &Aliases) -> Vec<(usize, usize)> {
        (1..5).map(|i| (a.destination + i * PAGE, PAGE)).collect()
    }

    #[test]
    fn real_whole_views_discard_private_cow_keep_source_outside_aliases_and_metadata() {
        let a = Aliases::new_pages(6, false);
        fill(&a);
        let selected = middle(&a);
        let (base, end) = (a.destination + PAGE, a.destination + 5 * PAGE);
        let cow = end - 4096;
        let mut old = 0;
        // SAFETY: the last host page of a selected, fixture-owned mapped view.
        assert_ne!(unsafe { VirtualProtect(cow as _, 4096, PAGE_WRITECOPY, &mut old) }, 0);
        unsafe { (cow as *mut u8).write_volatile(0xb4) };
        assert_eq!(unsafe { ((a.source + 5 * PAGE - 4096) as *const u8).read_volatile() }, 0x31);
        let outcome = {
            let mut p = physical();
            let _changing = victory_gpu::title_memory_remapping();
            let outcome = unmap_whole_views(&mut p, base, end, &selected, unmap_native).unwrap();
            assert!(selected.iter().all(|(at, _)| !p.views.contains_key(at)));
            assert!(p.shown.values().flatten().all(|&at| at < base || at >= end));
            outcome
        };
        assert_eq!((outcome.failures, outcome.panicked_chunks), (0, 0));
        for &(at, _) in &selected { assert_eq!(query(at as _).host.State, MEM_RESERVE); }
        for offset in [0, 5 * PAGE] {
            assert_eq!(query((a.destination + offset) as _).host.State, MEM_COMMIT);
            assert_eq!(unsafe { ((a.destination + offset) as *const u8).read_volatile() }, 0x31);
        }
        assert_eq!(unsafe { ((a.source + 5 * PAGE - 4096) as *const u8).read_volatile() }, 0x31);
    }

    #[test]
    fn partial_or_outside_range_declines_without_mutation_and_legacy_tails_survive() {
        let a = Aliases::new_pages(6, true);
        fill(&a);
        let selected = middle(&a);
        let (base, end) = (a.destination + PAGE, a.destination + 5 * PAGE);
        let calls = AtomicUsize::new(0);
        {
            let mut p = physical();
            let _changing = victory_gpu::title_memory_remapping();
            assert!(unmap_whole_views(&mut p, base, end, &selected, |_, _| {
                calls.fetch_add(1, Relaxed); true
            }).is_none(), "advertised PAGE views disagree with the actual single large view");
            assert!(unmap_whole_views(&mut p, base + PAGE, end, &selected, |_, _| {
                calls.fetch_add(1, Relaxed); true
            }).is_none(), "first view falls outside the requested range");
            assert_eq!(p.views[&a.destination].0, 6 * PAGE);
        }
        assert_eq!(calls.load(Relaxed), 0);
        assert_eq!(XMemMapPhysicalPages(base as _, 4, std::ptr::null()) as usize, base);
        for offset in [0, 5 * PAGE] {
            assert_eq!(query((a.destination + offset) as _).host.State, MEM_COMMIT);
            assert_eq!(unsafe { ((a.destination + offset) as *const u8).read_volatile() }, 0x31);
        }
        for &(at, _) in &selected { assert_eq!(query(at as _).host.State, MEM_RESERVE); }
    }

    #[test]
    fn injected_native_false_keeps_legacy_removed_metadata_and_explicit_owned_cleanup() {
        struct Cleanup(Option<usize>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Some(at) = self.0 { let _ = unmap_native(at, PAGE); }
            }
        }
        let a = Aliases::new_pages(6, false);
        fill(&a);
        let selected = middle(&a);
        let (base, end) = (a.destination + PAGE, a.destination + 5 * PAGE);
        let failed = base + PAGE;
        let mut cleanup = Cleanup(Some(failed));
        let calls = AtomicUsize::new(0);
        let outcome = {
            let mut p = physical();
            let _changing = victory_gpu::title_memory_remapping();
            let outcome = unmap_whole_views(&mut p, base, end, &selected, |at, len| {
                calls.fetch_add(1, Relaxed);
                at != failed && unmap_native(at, len)
            }).unwrap();
            assert!(selected.iter().all(|(at, _)| !p.views.contains_key(at)));
            assert!(p.shown.values().flatten().all(|&at| at < base || at >= end));
            outcome
        };
        assert_eq!((outcome.failures, outcome.panicked_chunks), (1, 0));
        assert_eq!(calls.load(Relaxed), 4, "failed native call is not retried");
        assert_eq!(query(failed as _).host.State, MEM_COMMIT, "injected failure left its owned view mapped");
        for &(at, _) in &selected {
            if at != failed { assert_eq!(query(at as _).host.State, MEM_RESERVE); }
        }
        assert!(unmap_native(failed, PAGE), "clean the deliberately untracked owned view");
        cleanup.0 = None;
        assert_eq!(query(failed as _).host.State, MEM_RESERVE);
        assert_eq!(unsafe { (a.source as *const u8).read_volatile() }, 0x31);
    }
}
