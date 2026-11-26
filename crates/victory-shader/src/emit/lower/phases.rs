//! Strict convergence and loop audits for the optional LDS phase candidate.

use super::{Atom, Exit, Fields, LaneExit, Program, Structured, Why,
    all_lane_blocks, lane_constant_masks, reads_mask_as_data};

thread_local! {
    /// See [`with_strict_phases`].
    pub(super) static STRICT_PHASES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Under [`with_strict_phases`]: the first breach of the contract in the last lowering.
    pub(super) static STRICT_FAULT: std::cell::Cell<Option<Why>> = const { std::cell::Cell::new(None) };
    /// Under [`with_strict_phases`]: the IR blocks the last lowering put a barrier in.
    pub(super) static STRICT_BARRIERS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs `f` (lowerings of a console wave64 program in two host waves of 32) under the strict
/// LDS phase contract (`crate::emit::try_lds_phases`): no meetings around lane branches
/// ([`lds_meetings`]), so LDS accesses where not every lane runs ask for their branches to be
/// taken wave-wide; and every barrier a lowering puts in DXIL (the translation's meetings, the
/// one before each LDS access, the exchanges of wave-wide branches, `wave_base`, the title's
/// `s_barrier`) must wait for the group and be run by every thread together: in the entry
/// block, or in an IR block every lane runs, outside any lane guard. A barrier left out or
/// kept as a fence, or an LDS access without its barrier, is a breach too.
/// Returns `f`'s result, the first breach of the last lowering inside it (none: kept), and the
/// IR blocks that lowering put barriers in (for [`phase_loops`]). The state before comes back
/// however `f` ends.
pub(in crate::emit) fn with_strict_phases<R>(f: impl FnOnce() -> R) -> (R, Option<Why>, Vec<usize>) {
    struct Restore(bool, Option<Why>, Vec<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            STRICT_PHASES.set(self.0);
            STRICT_FAULT.set(self.1);
            STRICT_BARRIERS.set(std::mem::take(&mut self.2));
        }
    }
    let _restore = Restore(STRICT_PHASES.replace(true), STRICT_FAULT.take(), STRICT_BARRIERS.take());
    let r = f();
    (r, STRICT_FAULT.get(), STRICT_BARRIERS.take())
}

pub(super) fn strict_breach(why: Why) {
    if STRICT_FAULT.get().is_none() {
        STRICT_FAULT.set(Some(why));
    }
}

/// The class of [`with_strict_phases`] (`crate::emit::try_lds_phases`): a program with no cross-lane
/// code of its own (lane reads and writes, `v_mbcnt`, permutes, swizzles, DPP, quad masks,
/// lane masks read as numbers, constant or opaque lane masks), so the only exchange between
/// the two host waves is the one wave-wide branches add; with one end and no indirect jump.
pub(in crate::emit) fn phase_class(p: &Program, st: &Structured) -> Result<(), Why> {
    let cross = p.blocks.iter().flat_map(|b| &b.insts).any(|i| {
        let n = i.name();
        n.starts_with("ds_swizzle")
            || n.contains("permute")
            || n.starts_with("v_mbcnt")
            || matches!(n, "v_readlane_b32" | "v_writelane_b32" | "v_readfirstlane_b32")
            || n.starts_with("s_wqm")
            || n.starts_with("s_quadmask")
            || matches!(n, "s_memtime" | "s_memrealtime" | "s_getreg_b32")
            || matches!(&i.fields, Fields::Valu(v) if v.dpp.is_some())
            || reads_mask_as_data(st, i)
    });
    if cross || lane_constant_masks(st) {
        return Err("cross-lane or timing-sensitive code in a strict LDS phase program");
    }
    let ends = p.blocks.iter().filter(|b| b.exit == Exit::End).count();
    if ends != 1 || p.blocks.iter().any(|b| b.exit == Exit::Indirect) {
        return Err("a strict LDS phase program without exactly one end");
    }
    Ok(())
}

/// The loops of the strict LDS phase contract, on the structure `st` the program was lowered
/// with and the IR blocks `barriers` that lowering put barriers in: no loop closed by a plain
/// jump (a thread at the iteration limit leaves the program, `abort_block`), no irreducible
/// loop, no lane leaving for an end block alone; and every loop that holds a barrier is
/// entered from a block every lane runs and has only wave-wide or uniform branches in it, no
/// end. Every thread then runs each iteration, meets each barrier of it, counts the same
/// iterations ([`LOOP_LIMIT`]) and leaves with the others.
pub(in crate::emit) fn phase_loops(p: &Program, st: &Structured, barriers: &[usize]) -> Result<(), Why> {
    let n = st.exits.len();
    let all = all_lane_blocks(st).ok_or("an indirect exit")?;
    let order = crate::ir::reverse_postorder(&p.blocks);
    let mut rpo = vec![usize::MAX; n];
    for (k, &b) in order.iter().enumerate() {
        rpo[b] = k;
    }
    let idom = crate::ir::idoms(&p.blocks, &order, &rpo);
    let dominates = |h: usize, mut b: usize| loop {
        if b == h {
            break true;
        }
        if idom[b] == b || idom[b] == usize::MAX {
            break false;
        }
        b = idom[b];
    };
    let succ = |b: usize| match st.exits[b] {
        LaneExit::Jump(t) => vec![t],
        LaneExit::Branch { taken, not_taken, .. } => vec![taken, not_taken],
        LaneExit::End | LaneExit::Indirect => Vec::new(),
    };
    let mut preds = vec![Vec::new(); n];
    for b in (0..n).filter(|&b| rpo[b] != usize::MAX) {
        for t in succ(b) {
            preds[t].push(b);
        }
    }
    let uniform = |x: usize| match st.exits[x] {
        LaneExit::Branch { when, .. } => {
            st.wide.contains(&x) || st.bools.support(when).iter().all(|a| matches!(a, Atom::Uniform(_)))
        }
        _ => true,
    };
    for x in (0..n).filter(|&x| rpo[x] != usize::MAX) {
        // Audit the final structure too: path simplification can change after
        // widening. No lane-dependent decision may split the source wave's PC.
        if !uniform(x) {
            return Err("a lane-dependent branch left unwidened in a strict LDS phase program");
        }
    }
    for b in (0..n).filter(|&b| rpo[b] != usize::MAX) {
        for t in succ(b).into_iter().filter(|&t| rpo[t] <= rpo[b]) {
            if matches!(st.exits[b], LaneExit::Jump(_)) {
                return Err("a loop closed by a plain jump in a strict LDS phase program");
            }
            if !dominates(t, b) {
                return Err("an irreducible loop in a strict LDS phase program");
            }
            // the loop of this back edge: what reaches `b` without passing its header `t`
            let mut body = vec![false; n];
            body[t] = true;
            let mut stack = vec![b];
            while let Some(x) = stack.pop() {
                if !std::mem::replace(&mut body[x], true) {
                    stack.extend(preds[x].iter().copied());
                }
            }
            if !barriers.iter().any(|&x| body.get(x).copied().unwrap_or(false)) {
                continue;
            }
            if !all[t] {
                return Err("a loop with barriers that not every thread enters");
            }
            for x in (0..n).filter(|&x| body[x]) {
                match st.exits[x] {
                    LaneExit::End | LaneExit::Indirect => {
                        return Err("a loop with barriers that a thread can leave the program from");
                    }
                    LaneExit::Branch { .. } if !uniform(x) => {
                        return Err("a loop with barriers that a thread can leave or repeat alone");
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

impl super::Lower<'_> {
    /// A scalar wait belongs to the source wave, not each host half separately.
    /// Conservatively finish all memory requests even for a partial/LGKM wait.
    pub(super) fn phase_wait(&mut self) {
        self.strict_barrier(self.cur);
        let mode = self.s.m.i32(1 | 2 | 8);
        let void = self.s.t.void;
        self.s.op(self.cur, "barrier", super::Attr::NoDuplicate, void, 80, &[mode]);
    }

    /// Under [`with_strict_phases`]: a barrier about to go into DXIL block `block` waits for
    /// the group and every thread runs it together: in the entry block before anything
    /// branches (`wave_base`), or as the current block of an IR block every lane runs, outside
    /// any lane guard. Else a breach; the IR block is noted for [`phase_loops`].
    pub(super) fn strict_barrier(&mut self, block: usize) {
        if !STRICT_PHASES.get() {
            return;
        }
        if crate::emit::group_sync() == 0 {
            return strict_breach("a phase barrier without its wait (VICTORY_GROUP_SYNC=0)");
        }
        if block == self.s.entry && block != self.cur {
            // (once, unless the entry block heads a loop)
            if self.counters.keys().any(|&h| self.block[h] == Some(self.s.entry)) {
                strict_breach("a phase barrier in an entry block that heads a loop");
            }
            return;
        }
        let every = all_lane_blocks(self.x.st).is_some_and(|all| all.get(self.cur_ir).copied().unwrap_or(false));
        if block != self.cur || self.guard_depth != 0 || !every {
            return strict_breach("a phase barrier not every thread of the group reaches together");
        }
        let b = self.cur_ir;
        STRICT_BARRIERS.with_borrow_mut(|v| {
            if !v.contains(&b) {
                v.push(b);
            }
        });
    }

}
