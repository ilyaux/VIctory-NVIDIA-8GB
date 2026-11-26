//! Actual Windows CPU tests for repeating one backing page, including rollback.
use super::alias_tests::Aliases;
use super::tests::query;
use super::*;
use windows_sys::Win32::System::Memory::{PAGE_WRITECOPY, VirtualProtect};

fn seeded(n: usize, large: bool) -> Aliases {
    let a = Aliases::new_pages(n, large);
    for i in 0..n {
        // SAFETY: the fixture owns these committed source pages and aliases.
        unsafe { ((a.source + i * PAGE) as *mut u8).write_bytes((i + 1) as u8, PAGE) };
    }
    a
}

fn run(a: &Aliases, make: impl FnMut(&mut Physical, usize, usize, Slice) -> bool) -> Option<bool> {
    let mut p = physical();
    let _changing = victory_gpu::title_memory_remapping();
    repeat_alias_batch(
        &mut p,
        a.destination,
        a.source + (a.pages.len() - 1) * PAGE,
        a.pages.len() * PAGE,
        make,
    )
}

/// Prefix pages show the last source page; the suffix keeps its original Slice.
/// On a second restoration failure the suffix is explicitly observed as holes.
fn layout(a: &Aliases, prefix: usize, hole: Option<usize>, lost_suffix: bool) {
    let n = a.pages.len();
    let missing = |i| hole.is_some_and(|h| i == h || lost_suffix && i > h);
    let wanted = |i| if i < prefix { n - 1 } else { i };
    let p = physical();
    let _reading = victory_gpu::title_memory_reading();
    let origin = &p.views[&a.source].1;
    for i in 0..n {
        let at = a.destination + i * PAGE;
        let view = p
            .views
            .range(..=at)
            .next_back()
            .filter(|&(base, (len, _))| at - *base < *len);
        if missing(i) {
            assert!(
                view.is_none(),
                "hole {i} must not have phantom Physical metadata"
            );
            assert_eq!(query(at as _).host.State, MEM_RESERVE);
        } else {
            let (&base, (_, slice)) = view.expect("mapped prefix/suffix");
            assert!(Arc::ptr_eq(&slice.section, &origin.section));
            assert_eq!(slice.index + (at - base) / PAGE, origin.index + wanted(i));
            assert_eq!(query(at as _).host.State, MEM_COMMIT);
            // SAFETY: the OS query and tracked view both say this owned page is mapped.
            assert_eq!(
                unsafe { (at as *const u8).read_volatile() },
                (wanted(i) + 1) as u8
            );
        }
    }
    for i in 0..n {
        let mut expected = vec![a.source + i * PAGE];
        expected.extend(
            (0..n)
                .filter(|&k| !missing(k) && wanted(k) == i)
                .map(|k| a.destination + k * PAGE),
        );
        expected.sort_unstable();
        let mut actual = p.shown.get(&origin.key(i)).cloned().unwrap_or_default();
        actual.sort_unstable();
        assert_eq!(
            actual, expected,
            "reverse aliases of backing page {i}; no duplicates or lost tail"
        );
    }
}

#[test]
fn whole_view_repeats_the_last_source_page_instead_of_consecutive_pages() {
    let a = seeded(4, true);
    let last = physical().views[&a.source].1.index + 3;
    let mut calls = Vec::new();
    assert_eq!(
        run(&a, |p, at, len, slice| {
            calls.push((at, len, slice.index));
            map(p, at, len, slice)
        }),
        Some(true)
    );
    assert_eq!(
        calls,
        (0..4)
            .map(|i| (a.destination + i * PAGE, PAGE, last))
            .collect::<Vec<_>>()
    );
    layout(&a, 4, None, false);
    // SAFETY: all four destinations must remain aliases of this one source page.
    unsafe {
        ((a.source + 3 * PAGE + 17) as *mut u8).write_volatile(0x79);
        for i in 0..4 {
            assert_eq!(
                ((a.destination + i * PAGE + 17) as *const u8).read_volatile(),
                0x79
            );
        }
    }
}

#[test]
fn first_middle_and_last_mapping_failures_preserve_the_strict_old_suffix() {
    // Also exercise the placeholder topology left by a real split and by a
    // mapped view rolled back before registration (the commit-failure boundary).
    for stage in 0..3 {
        for failed_page in 0..4 {
            let a = seeded(4, true);
            let mut injected = false;
            assert_eq!(
                run(&a, |p, at, len, slice| {
                    if !injected && at == a.destination + failed_page * PAGE && len == PAGE {
                        injected = true;
                        if stage != 0 {
                            unsafe {
                                VirtualFree(at as _, len, MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER)
                            };
                        }
                        if stage == 2 {
                            let view = unsafe {
                                MapViewOfFile3(
                                    slice.section.0,
                                    GetCurrentProcess(),
                                    at as _,
                                    (slice.index * PAGE) as u64,
                                    len,
                                    MEM_REPLACE_PLACEHOLDER,
                                    PAGE_READWRITE,
                                    std::ptr::null_mut(),
                                    0,
                                )
                            };
                            assert_eq!(view.Value as usize, at);
                            assert_ne!(
                                unsafe {
                                    UnmapViewOfFile2(
                                        GetCurrentProcess(),
                                        view,
                                        MEM_PRESERVE_PLACEHOLDER,
                                    )
                                },
                                0
                            );
                        }
                        false
                    } else {
                        map(p, at, len, slice)
                    }
                }),
                Some(false)
            );
            assert!(injected);
            layout(&a, failed_page, Some(failed_page), false);
            if failed_page < 3 {
                let p = physical();
                let (len, tail) = &p.views[&(a.destination + (failed_page + 1) * PAGE)];
                assert_eq!(*len, (3 - failed_page) * PAGE);
                assert_eq!(tail.index, p.views[&a.source].1.index + failed_page + 1);
            }
        }
    }
}

#[test]
fn rollback_discards_the_whole_old_views_private_cow_as_the_first_legacy_unmap_does() {
    let a = seeded(4, true);
    let mut old = 0;
    let at = a.destination + 2 * PAGE;
    assert_ne!(
        unsafe { VirtualProtect(at as _, PAGE, PAGE_WRITECOPY, &mut old) },
        0
    );
    unsafe { (at as *mut u8).write_volatile(0xee) };
    assert_eq!(
        unsafe { (a.source as *const u8).add(2 * PAGE).read_volatile() },
        3
    );
    let mut injected = false;
    assert_eq!(
        run(&a, |p, at, len, slice| {
            if !injected && at == a.destination + PAGE && len == PAGE {
                injected = true;
                false
            } else {
                map(p, at, len, slice)
            }
        }),
        Some(false)
    );
    layout(&a, 1, Some(1), false);
}

#[test]
fn ineligible_separate_page_views_keep_private_cow_untouched() {
    let a = seeded(4, false);
    let at = a.destination + 2 * PAGE;
    let mut old = 0;
    assert_ne!(
        unsafe { VirtualProtect(at as _, PAGE, PAGE_WRITECOPY, &mut old) },
        0
    );
    unsafe { (at as *mut u8).write_volatile(0xee) };
    assert_eq!(run(&a, map), None);
    assert_eq!(unsafe { (at as *const u8).read_volatile() }, 0xee);
    assert_eq!(
        unsafe { ((a.source + 2 * PAGE) as *const u8).read_volatile() },
        3
    );
}

#[test]
fn public_repeated_mapper_keeps_success_and_legacy_source_failure_outcomes() {
    {
        let a = seeded(4, true);
        assert!(map_alias_repeated(
            a.destination as u64,
            (a.source + 3 * PAGE) as u64,
            (4 * PAGE) as u64
        ));
        layout(&a, 4, None, false);
    }
    for kind in 0..3 {
        let a = seeded(4, true);
        let from = match kind {
            0 => a.reservations[0],
            1 => a.destination,
            _ => 0,
        };
        assert_eq!(
            map_alias_repeated(a.destination as u64, from as u64, (4 * PAGE) as u64),
            kind == 2
        );
        layout(&a, 0, Some(0), kind == 2);
    }
}

#[test]
fn separate_old_page_views_are_ineligible_without_mutation() {
    let a = seeded(4, false);
    let mut made = 0;
    assert_eq!(
        run(&a, |p, at, len, slice| {
            made += 1;
            map(p, at, len, slice)
        }),
        None
    );
    assert_eq!(made, 0);
    layout(&a, 0, None, false);
    let p = physical();
    for i in 0..4 {
        assert_eq!(p.views[&(a.destination + i * PAGE)].0, PAGE);
    }
}

#[test]
fn failed_suffix_restoration_is_not_reported_as_success() {
    let a = seeded(4, true);
    let mut failures = 0;
    assert_eq!(
        run(&a, |p, at, len, slice| {
            if (at == a.destination + PAGE && len == PAGE)
                || (at == a.destination + 2 * PAGE && len == 2 * PAGE)
            {
                failures += 1;
                false
            } else {
                map(p, at, len, slice)
            }
        }),
        Some(false)
    );
    assert_eq!(
        failures, 2,
        "requested map and strict-suffix restore both failed"
    );
    // This is the documented host-failure limit, not a claim that rollback succeeded.
    layout(&a, 1, Some(1), true);
}

#[test]
#[ignore = "manual CPU: VICTORY_FRAME_LOG=1"]
fn production_operation_counts_remove_quadratic_tail_work() {
    use victory_gpu::framelog::{self, T};
    assert!(
        framelog::on(),
        "enable frame counters before fixture allocation"
    );
    assert!(
        std::env::var("VICTORY_ALIAS_REUSE").as_deref() != Ok("1"),
        "run alone with reuse off; do not skip the legacy final page"
    );
    let totals = || {
        [
            T::PhysicalMapCalls,
            T::PhysicalUnmapCalls,
            T::PhysicalTailPages,
        ]
        .map(framelog::total)
    };
    let delta = |before: [u64; 3]| {
        let after = totals();
        std::array::from_fn::<_, 3, _>(|i| after[i] - before[i])
    };
    for n in [4, 8] {
        {
            let a = seeded(n, true);
            let before = totals();
            let source = (a.source + (n - 1) * PAGE) as u64;
            assert!((0..n).all(|i| map_alias(
                (a.destination + i * PAGE) as u64,
                source,
                PAGE as u64
            )));
            assert_eq!(
                delta(before),
                [(2 * n - 1) as u64, n as u64, (n * (n - 1) / 2) as u64]
            );
            layout(&a, n, None, false);
        }
        {
            let a = seeded(n, true);
            let before = totals();
            assert_eq!(run(&a, map), Some(true));
            assert_eq!(delta(before), [n as u64, 1, 0]);
            layout(&a, n, None, false);
        }
    }
}
