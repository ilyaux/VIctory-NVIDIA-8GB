//! Actual Windows CPU mappings; no adapter, D3D device, or game is involved.

use super::*;
use super::tests::{allocate, query};
use windows_sys::Win32::System::Memory::{
    MEM_MAPPED, PAGE_READONLY, PAGE_WRITECOPY, VirtualProtect,
};

const MEM_COALESCE_PLACEHOLDERS: u32 = 1; // winnt.h; grouped under SystemServices in windows-sys.

const BYTES: usize = 2 * PAGE;
pub(super) struct Aliases {
    pub(super) pages: Vec<usize>,
    pub(super) reservations: Vec<usize>,
    pub(super) source: usize,
    pub(super) destination: usize,
}

impl Aliases {
    fn new(large_destination: bool) -> Self {
        Self::new_pages(2, large_destination)
    }

    pub(super) fn new_pages(count: usize, large_destination: bool) -> Self {
        let mut a = Self { pages: allocate(count), reservations: Vec::new(), source: 0, destination: 0 };
        let bytes = count * PAGE;
        let reserved = bytes + 2 * PAGE;
        for _ in 0..2 {
            let at = XMemVirtualAlloc(std::ptr::null_mut(), reserved, MEM_RESERVE | PAGES_64K, 0, PAGE_READWRITE);
            assert!(!at.is_null());
            a.reservations.push(at as usize);
        }
        // Padding keeps shown_at runs distinct even for adjacent reservations.
        a.source = a.reservations[0] + PAGE;
        a.destination = a.reservations[1] + PAGE;
        assert_eq!(XMemMapPhysicalPages(a.source as _, count, a.pages.as_ptr()) as usize, a.source);
        if large_destination {
            assert_eq!(XMemMapPhysicalPages(a.destination as _, count, a.pages.as_ptr()) as usize, a.destination);
        } else {
            assert!(map_alias(a.destination as u64, a.source as u64, bytes as u64));
        }
        a
    }

    fn warm(&self) {
        let bytes = self.pages.len() * PAGE;
        // SAFETY: both ranges are our committed aliases. Only the fixture faults
        // pages in; the production resident_shared query must never do that.
        unsafe {
            (self.source as *mut u8).write_bytes(0x31, bytes);
            for offset in (0..bytes).step_by(4096) {
                assert_eq!(((self.destination + offset) as *const u8).read_volatile(), 0x31);
            }
        }
        assert!(resident_shared(self.destination, bytes), "fixture must actually be resident and shared");
    }

    fn apply(&self, to: usize, from: usize, bytes: usize) -> (bool, bool) {
        let reused = {
            let p = physical();
            let _changing = victory_gpu::title_memory_remapping();
            reuse_alias(&p, to, from, bytes)
        };
        // A failed attempt may already have reset protection/notifications.
        // Always take the public production fallback before making assertions.
        let mapped = reused || map_alias(to as u64, from as u64, bytes as u64);
        (reused, mapped)
    }
}

impl Drop for Aliases {
    fn drop(&mut self) {
        let reserved = (self.pages.len() + 2) * PAGE;
        for &at in &self.reservations {
            XMemMapPhysicalPages(at as _, reserved / PAGE, std::ptr::null());
        }
        XMemFreePhysicalPages(self.pages.len(), self.pages.as_ptr());
        for &at in &self.reservations {
            // SAFETY: only this fixture's fully unmapped placeholder reservation.
            // Coalescing may fail harmlessly when it is already one placeholder.
            unsafe {
                VirtualFree(at as _, reserved, MEM_RELEASE | MEM_COALESCE_PLACEHOLDERS);
                VirtualFree(at as _, 0, MEM_RELEASE);
            }
        }
    }
}

#[test]
fn identical_two_page_aliases_reset_readonly_and_keep_reverse_entries_unique() {
    let a = Aliases::new(false);
    a.warm();
    let before = shown_at(&a.pages, 0, BYTES);
    assert_eq!(before.len(), 2, "one source run and one destination run");
    let mut old = 0;
    // SAFETY: exactly one live destination view owned by this fixture.
    assert_ne!(unsafe { VirtualProtect(a.destination as _, PAGE, PAGE_READONLY, &mut old) }, 0);
    for _ in 0..8 {
        assert_eq!(a.apply(a.destination, a.source, BYTES), (true, true));
        assert_eq!(shown_at(&a.pages, 0, BYTES), before, "no duplicate shown addresses");
        assert_eq!(query(a.destination as _).host.Protect, PAGE_READWRITE);
        let p = physical();
        assert_eq!(p.views[&a.destination].0, PAGE);
        assert_eq!(p.views[&(a.destination + PAGE)].0, PAGE);
    }
    // SAFETY: writing the alias must still reach the source's actual section.
    unsafe {
        ((a.destination + BYTES - 1) as *mut u8).write_volatile(0x52);
        assert_eq!(((a.source + BYTES - 1) as *const u8).read_volatile(), 0x52);
    }
}

#[test]
fn private_cow_in_last_host_page_requires_real_remapping_not_just_readwrite() {
    let a = Aliases::new(false);
    a.warm();
    let offset = BYTES - 4096;
    let at = a.destination + offset;
    let mut old = 0;
    // SAFETY: one committed host page inside this fixture's mapped view.
    assert_ne!(unsafe { VirtualProtect(at as _, 4096, PAGE_WRITECOPY, &mut old) }, 0);
    unsafe { (at as *mut u8).write_volatile(0xb4) };
    assert_eq!(query(at as _).host.Protect, PAGE_READWRITE, "COW write becomes private RW");
    assert_eq!(query(at as _).host.Type, MEM_MAPPED, "VirtualQuery cannot identify private COW");
    assert_eq!(unsafe { ((a.source + offset) as *const u8).read_volatile() }, 0x31);
    assert!(!resident_shared(a.destination, BYTES), "check all host pages, including the last");
    assert_eq!(a.apply(a.destination, a.source, BYTES), (false, true));
    assert_eq!(unsafe { (at as *const u8).read_volatile() }, 0x31, "fallback must discard private bytes");
    unsafe {
        ((a.source + offset) as *mut u8).write_volatile(0x67);
        assert_eq!((at as *const u8).read_volatile(), 0x67, "destination rejoins shared backing");
    }
}

#[test]
fn ineligible_shapes_keep_partial_view_and_legacy_failure_behavior() {
    {
        let a = Aliases::new(true);
        a.warm();
        assert_eq!(a.apply(a.destination, a.source, PAGE), (false, true));
        assert_eq!(unsafe { ((a.destination + PAGE) as *const u8).read_volatile() }, 0x31, "tail survives");
    }
    for overlap in [false, true] {
        let a = Aliases::new(false);
        let from = if overlap { a.destination } else { a.reservations[0] }; // owned unmapped padding
        assert_eq!(a.apply(a.destination, from, PAGE), (false, false));
        assert_eq!(query(a.destination as _).host.State, MEM_RESERVE, "old path unmaps before source failure");
        assert_eq!(query((a.destination + PAGE) as _).host.State, MEM_COMMIT, "other page is unchanged");
    }
    // Pure preflight overflow: never pass bogus addresses to the legacy VM path.
    let p = physical();
    let _changing = victory_gpu::title_memory_remapping();
    let top = usize::MAX & !(PAGE - 1);
    assert!(!reuse_alias(&p, top, PAGE, PAGE));
    assert!(!reuse_alias(&p, PAGE, top, PAGE));
}

#[test]
fn residency_query_rejects_owned_uncommitted_pages_without_faulting_them_in() {
    let a = Aliases::new(false);
    let untouched = a.reservations[0];
    assert_eq!(query(untouched as _).host.State, MEM_RESERVE);
    assert!(!resident_shared(untouched, PAGE));
    assert!(!resident_shared(untouched, 0));
    assert_eq!(query(untouched as _).host.State, MEM_RESERVE);
}
