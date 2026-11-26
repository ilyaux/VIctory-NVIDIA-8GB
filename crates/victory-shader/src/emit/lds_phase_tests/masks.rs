//! Original synthetic canary for one source Wave64 on 32-lane host waves: a logical EXEC of
//! high lanes only ({32, 35, 63}) built from lane compares and mask logic, its first set bit,
//! bit count and `v_mbcnt` prefixes, one elected-lane UAV atomic whose return is broadcast
//! with `v_readfirstlane` to the whole active set and from there, through an SGPR, to both
//! host halves. A half-only ballot, a wrong high-half lane offset, a first-lane read that
//! picks the (empty) low half, or a lost atomic return all change a checked field.
//!
//! These tests cover the opt-in exact mask/prefix/first-lane admission in `try_lds_phases`.

use super::*;

/// Dwords of the checked UAV: 64 broadcast values, 64 prefixes (+1, 0 = not active), 64
/// copies of `ff1 << 8 | count`, 64 values read back from LDS, then the counter.
const WORDS: usize = 320;
/// Byte offsets of the fields after the first.
const PREFIXES: u32 = 64 * 4;
const SCALARS: u32 = 128 * 4;
const HANDOFF: u32 = 192 * 4;
const COUNTER: u32 = 256 * 4;
/// The counter's first value, stored by logical lane 0 before the election.
const START: u32 = 100;
/// The logical EXEC of the active section.
const ACTIVE: [usize; 3] = [32, 35, 63];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    /// 64 x 1 x 1: the native x id is the flat id, but the program does not use it.
    Row,
    /// 8 x 8 x 1: v0 is x and v1 is y; the program takes its flat id from a full-mask
    /// `v_mbcnt` instead (and reuses v1 as the LDS address).
    Square,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Consumer {
    /// `s_ff1`/`s_bcnt1` of EXEC inside the active section; the election compares the lane
    /// id with that first set bit.
    Exec,
    /// The mask captured before EXEC changed, counted after EXEC was narrowed to the elected
    /// lane: its bits outside the current EXEC must still count (3, not 1).
    SavedMask,
    /// The complement of the captured mask (lane 0 first, 61 lanes): exact or refused.
    Complement,
    /// EXEC copied inside the active section and counted after the section joined (a mask
    /// phi with 0 on the skipping edge): exact or refused.
    Joined,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Source {
    /// Lane compares and `s_or_b64`: the admitted form.
    Compares,
    /// The mask is the root constants s[0:1]: opaque, must stay refused.
    Opaque,
    /// The mask is the inline constant 9 (lanes 0 and 3): a constant lane mask, must stay
    /// refused.
    Constant,
}

fn vop3(name: &str, dst: u8, a: u16, b: u16) -> [u32; 2] {
    [0xd400_0000 | opcode(Encoding::Vop3, name) << 16 | u32::from(dst), u32::from(a) | u32::from(b) << 9]
}
fn sop2(name: &str, dst: u8, a: u8, b: u8) -> u32 {
    0x8000_0000 | opcode(Encoding::Sop2, name) << 23 | u32::from(dst) << 16 | u32::from(b) << 8 | u32::from(a)
}
/// `v_readfirstlane_b32 s<dst>, v<src>` (VOP1, scalar destination).
fn readfirstlane(dst: u8, src: u8) -> u32 {
    0x7e00_0000 | u32::from(dst) << 17 | opcode(Encoding::Vop1, "v_readfirstlane_b32") << 9 | (256 + u32::from(src))
}
fn mov_literal(dst: u8, value: u32) -> [u32; 2] { [mov(dst, 255), value] }
fn add_literal(dst: u8, value: u32, src: u8) -> [u32; 2] { [vop2("v_add_nc_u32", dst, 255, src), value] }
/// `s_waitcnt 0`: every counter.
fn wait_all() -> u32 { 0xbf80_0000 | opcode(Encoding::Sopp, "s_waitcnt") << 16 }

const VCC: u8 = 106;
const EXEC_LO: u8 = 126;
const EXEC_HI: u16 = 127;
const ZERO: u8 = 128;
const MINUS_ONE: u16 = 193;
const fn int(n: u8) -> u8 { 128 + n }
const fn v(n: u8) -> u16 { 256 + n as u16 }

fn mask_program(shape: Shape, consumer: Consumer) -> NativeProgram {
    program(shape, consumer, Source::Compares)
}

fn program(shape: Shape, consumer: Consumer, source: Source) -> NativeProgram {
    let mut c = Code::default();
    // s[8:11] = the raw UAV descriptor from the table at s4 (s5 = 0), ready before use.
    c.put([sop1("s_mov_b32", 5, ZERO), 0x3d << 26 | 0x02 << 18 | 8 << 6 | 2, 125 << 25, wait_all()]);
    // Every register read later is defined first: v1 LDS address, v2 LDS data, v6 atomic
    // data/return, and the scalars the tail reads on every path.
    c.put([mov(1, u16::from(ZERO)), mov(2, u16::from(ZERO)), mov(6, u16::from(ZERO))]);
    c.put([sop1("s_mov_b32", 24, ZERO), sop1("s_mov_b32", 25, ZERO), sop1("s_mov_b32", 30, ZERO),
        sop1("s_mov_b64", 36, ZERO)]);
    // v10 = the logical flat id (the console lane), from a full-mask prefix: not v0, which is
    // only x in an 8 x 8 group.
    c.put(vop3("v_mbcnt_lo_u32_b32", 10, MINUS_ONE, u16::from(ZERO)));
    c.put(vop3("v_mbcnt_hi_u32_b32", 10, MINUS_ONE, v(10)));
    // The counter starts at 100, stored by logical lane 0 alone, visible before the election.
    c.put([cmp("v_cmp_eq_u32", u16::from(ZERO), 10), sop1("s_and_saveexec_b64", 16, VCC)]);
    c.branch(0xbf88_0000, "init_done");
    c.put(mov_literal(6, START));
    c.put(mov_literal(7, COUNTER));
    c.put(buffer("buffer_store_dword", 6, false));
    c.label("init_done");
    c.put([sop1("s_mov_b64", EXEC_LO, 16), wait_stores(), mov(6, u16::from(ZERO))]);
    // s[20:21] = the logical mask, made where every lane runs.
    match source {
        Source::Compares => {
            c.put([cmp("v_cmp_eq_u32", u16::from(int(32)), 10), sop1("s_mov_b64", 20, VCC)]);
            c.put([cmp("v_cmp_eq_u32", u16::from(int(35)), 10), sop2("s_or_b64", 20, 20, VCC)]);
            c.put([cmp("v_cmp_eq_u32", u16::from(int(63)), 10), sop2("s_or_b64", 20, 20, VCC)]);
        }
        Source::Opaque => c.put([sop1("s_mov_b64", 20, 0)]),
        Source::Constant => c.put([sop1("s_mov_b64", 20, int(9))]),
    }
    if consumer == Consumer::Complement {
        c.put([sop1("s_not_b64", 32, 20)]);
    }
    c.put([sop1("s_and_saveexec_b64", 22, 20)]);
    c.branch(0xbf88_0000, "active_done");
    match consumer {
        Consumer::Exec => c.put([sop1("s_ff1_i32_b64", 24, EXEC_LO), sop1("s_bcnt1_i32_b64", 25, EXEC_LO)]),
        Consumer::Complement => c.put([sop1("s_ff1_i32_b64", 24, 32), sop1("s_bcnt1_i32_b64", 25, 32)]),
        Consumer::SavedMask | Consumer::Joined => {}
    }
    // v11 = this lane's prefix in EXEC; prefix + 1 at PREFIXES + 4 * lane.
    c.put(vop3("v_mbcnt_lo_u32_b32", 11, u16::from(EXEC_LO), u16::from(ZERO)));
    c.put(vop3("v_mbcnt_hi_u32_b32", 11, EXEC_HI, v(11)));
    c.put([vop2("v_add_nc_u32", 12, u16::from(int(1)), 11), vop2("v_lshlrev_b32", 7, u16::from(int(2)), 10)]);
    c.put(add_literal(7, PREFIXES, 7));
    c.put(buffer("buffer_store_dword", 12, false));
    // The election: the lane at the first set bit, or the lane whose prefix is 0.
    c.put([match consumer {
        Consumer::Exec => cmp("v_cmp_eq_u32", 24, 10),
        _ => cmp("v_cmp_eq_u32", u16::from(ZERO), 11),
    }, sop1("s_and_saveexec_b64", 28, VCC)]);
    c.branch(0xbf88_0000, "elect_done");
    if consumer == Consumer::SavedMask {
        // EXEC is the elected lane now; the mask made before still has all three.
        c.put([sop1("s_ff1_i32_b64", 24, 20), sop1("s_bcnt1_i32_b64", 25, 20)]);
    }
    c.put([mov(6, u16::from(int(1)))]);
    c.put(mov_literal(7, COUNTER));
    c.put(buffer("buffer_atomic_add", 6, true));
    c.put([wait_all()]);
    c.label("elect_done");
    c.put([sop1("s_mov_b64", EXEC_LO, 28)]);
    if consumer == Consumer::Joined {
        c.put([sop1("s_mov_b64", 36, EXEC_LO)]);
    }
    // The elected lane's return, for the active set and (through s30) for every lane.
    c.put([readfirstlane(30, 6), mov(2, 30)]);
    // Disjoint writes; the tail reads the elected high lane's value across both halves.
    c.put([vop2("v_lshlrev_b32", 1, u16::from(int(2)), 10)]);
    c.put(ds("ds_write_b32", 0, 0));
    c.label("active_done");
    c.put([sop1("s_mov_b64", EXEC_LO, 22)]);
    if consumer == Consumer::Joined {
        c.put([sop1("s_ff1_i32_b64", 24, 36), sop1("s_bcnt1_i32_b64", 25, 36)]);
    }
    // Every lane: the broadcast, ff1 << 8 | count, and the LDS handoff.
    c.put([mov(14, 30), vop2("v_lshlrev_b32", 7, u16::from(int(2)), 10)]);
    c.put(buffer("buffer_store_dword", 14, false));
    c.put([sop2("s_lshl_b32", 31, 24, int(8)), sop2("s_or_b32", 31, 31, 25), mov(15, 31)]);
    c.put(add_literal(7, SCALARS, 7));
    c.put(buffer("buffer_store_dword", 15, false));
    c.put(mov_literal(1, 4 * ACTIVE[0] as u32));
    c.put(ds("ds_read_b32", 0, 16));
    c.put(add_literal(7, HANDOFF - SCALARS, 7));
    c.put(buffer("buffer_store_dword", 16, false));
    c.put([0xbf81_0000]);
    let (rsrc2, threads) = match shape {
        Shape::Row => (5 << 1 | 1 << 15, [64, 1, 1]),
        Shape::Square => (5 << 1 | 1 << 15 | 1 << 11, [8, 8, 1]),
    };
    NativeProgram {
        stage: HwStage::Cs, wave64: true, vgprs: 24,
        layout: vec![
            RootParam { root: RootType::Constants, table_offset_dwords: 0, sgprs: 4, base_register: 0, user_data: 0, slot: 0 },
            RootParam { root: RootType::Table, table_offset_dwords: 0, sgprs: 1, base_register: 0, user_data: 4, slot: 1 },
        ],
        used: vec![UsedResource { kind: ResourceType::Uav, root: RootType::Table, table_offset_dwords: 0, lower_bound: 0, stride: 0, dwords: 4, user_data: 4, slot: 1 }],
        io: StageIo::Cs(ComputeIo { rsrc2, threads }),
        code: c.finish(),
    }
}

/// What the console computes, field by field, from the mask alone.
fn model(consumer: Consumer) -> Vec<u32> {
    let mask: u64 = ACTIVE.iter().map(|&l| 1u64 << l).sum();
    let counted = if consumer == Consumer::Complement { !mask } else { mask };
    let first = counted.trailing_zeros();
    let count = counted.count_ones();
    let mut counter = START;
    // the election: one lane, the first active one; its return is the old counter
    let elected = mask.trailing_zeros() as usize;
    assert_eq!(elected, 32);
    let old = counter;
    counter += 1;
    let mut out = vec![0u32; WORDS];
    for lane in 0..64usize {
        out[lane] = old;
        if mask >> lane & 1 != 0 {
            out[64 + lane] = (mask & ((1u64 << lane) - 1)).count_ones() + 1;
        }
        out[128 + lane] = first << 8 | count;
        out[192 + lane] = old;
    }
    out[256] = counter;
    out
}

fn expected(consumer: Consumer) -> Vec<u32> {
    let (first, count) = if consumer == Consumer::Complement { (0, 61) } else { (32, 3) };
    let mut out = vec![0u32; WORDS];
    for lane in 0..64 {
        out[lane] = START;
        out[128 + lane] = first << 8 | count;
        out[192 + lane] = START;
    }
    for (prefix, lane) in ACTIVE.into_iter().enumerate() {
        out[64 + lane] = prefix as u32 + 1;
    }
    out[256] = START + 1;
    out
}

const SHAPES: [Shape; 2] = [Shape::Row, Shape::Square];

#[test]
fn the_oracle_is_the_console_semantics_of_the_mask() {
    for consumer in [Consumer::Exec, Consumer::SavedMask, Consumer::Complement, Consumer::Joined] {
        assert_eq!(model(consumer), expected(consumer), "{consumer:?}");
    }
    let e = expected(Consumer::Exec);
    assert_eq!((e[64 + 32], e[64 + 35], e[64 + 63]), (1, 2, 3), "prefixes 0, 1, 2 (+1)");
    assert_eq!(e[128] >> 8, 32, "first set bit");
    assert_eq!(e[128] & 0xff, 3, "count");
    assert!(e[..64].iter().all(|&x| x == START) && e[192..256].iter().all(|&x| x == START), "both halves");
}

#[test]
fn the_fixtures_are_well_formed() {
    for shape in SHAPES {
        for consumer in [Consumer::Exec, Consumer::SavedMask, Consumer::Complement, Consumer::Joined] {
            for source in [Source::Compares, Source::Opaque, Source::Constant] {
                let native = program(shape, consumer, source);
                let p = crate::spill::build(&native).unwrap();
                assert_eq!(p.blocks.iter().filter(|b| b.exit == crate::ir::Exit::End).count(), 1, "one end");
                structure::structure(&p).expect("structured");
                io::raise(&p, &native).expect("inputs");
            }
        }
    }
}

/// Calibration: the canary covers code the ordinary lowering does not already run in waves of
/// 32 (a lane read and masks read as data inside lane branches).
#[test]
fn the_high_only_mask_needs_the_strict_candidate() {
    let native = mask_program(Shape::Row, Consumer::Exec);
    let p = crate::spill::build(&native).unwrap();
    let st = structure::structure(&p).unwrap();
    let res = resource::track(&p, &native.layout, native.stage, Some(&st)).unwrap();
    let io = io::raise(&p, &native).unwrap();
    let previous = lower::TEST_HOST_WAVE.replace(Some(32));
    let r = super::super::emit_program(&empty_container(), &native, &p, &st, &io, &res);
    lower::TEST_HOST_WAVE.set(previous);
    assert!(!matches!(r, Ok((_, Some(32)))), "the fixture must not already translate for 32-lane waves");
}

#[test]
fn high_only_mask_prefix_and_broadcast_are_admitted() {
    for shape in SHAPES {
        for consumer in [Consumer::Exec, Consumer::SavedMask] {
            let mut shader = phase_shader(&mask_program(shape, consumer))
                .unwrap_or_else(|e| panic!("{shape:?} {consumer:?}: exact mask/prefix/first-lane admission: {e}"));
            validate(&mut shader);
        }
    }
}

/// Complement and joined masks: the original bits must survive, or the program is refused.
/// Which of the two holds is the admission's decision; the manual device test checks the bits
/// of whatever is admitted.
#[test]
fn complement_and_joined_masks_are_exact_or_refused() {
    for shape in SHAPES {
        for consumer in [Consumer::Complement, Consumer::Joined] {
            if let Ok(mut shader) = phase_shader(&mask_program(shape, consumer)) {
                validate(&mut shader);
            }
        }
    }
}

#[test]
fn opaque_and_constant_lane_masks_stay_refused() {
    for shape in SHAPES {
        for source in [Source::Opaque, Source::Constant] {
            let r = phase_shader(&program(shape, Consumer::Exec, source));
            assert!(matches!(r, Err(ShaderError::Unsupported { .. })), "{shape:?} {source:?} must be refused");
        }
    }
}

#[test]
fn mask_programs_of_several_source_waves_are_refused() {
    let mut native = mask_program(Shape::Row, Consumer::Exec);
    if let StageIo::Cs(cs) = &mut native.io {
        cs.threads = [128, 1, 1];
    }
    assert!(phase_shader(&native).is_err());
}

fn dispatch_masks(host: &super::super::host::Host, shader: &HostShader, consumer: Consumer) {
    use super::super::host::{Dispatch, Root, View};
    let (root, pipeline) = host.compute_pipeline(shader).expect("explicit device pipeline");
    let out = host.dispatch(&root, &pipeline, &Dispatch {
        roots: vec![(0, Root::Constants(vec![0; shader.user_data_dwords().max(1) as usize]))],
        views: &[View::RawUav(WORDS as _)], samplers: &[], groups: [1, 1, 1],
    }).expect("bounded offscreen mask dispatch");
    assert_eq!(out.len(), 1);
    let (got, want) = (&out[0], expected(consumer));
    assert_eq!(got.len(), WORDS);
    assert_eq!(got[..64], want[..64], "{consumer:?}: the broadcast return in both host halves");
    assert_eq!(got[64..128], want[64..128], "{consumer:?}: prefixes of the active lanes only");
    assert_eq!(got[128..192], want[128..192], "{consumer:?}: first set bit and count in every lane");
    assert_eq!(got[192..256], want[192..256], "{consumer:?}: the LDS handoff in both host halves");
    assert_eq!(got[256], want[256], "{consumer:?}: exactly one reservation");
    assert_eq!(got[..], want[..], "{consumer:?}: the whole UAV");
}

#[test]
#[ignore = "manual: diagnostics, loop limit 16, named hardware device, VICTORY_LDS_PHASE_GPU_TESTS=1"]
fn high_only_masks_manual_on_explicit_device() {
    assert_eq!(std::env::var("VICTORY_LDS_PHASE_GPU_TESTS").as_deref(), Ok("1"));
    assert!(cfg!(feature = "diagnostics"), "a bounded diagnostics canary build is required");
    assert_eq!(super::super::loop_limit(), 16, "set VICTORY_SHADER_LOOP_LIMIT=16 before this process starts");
    // Every shader is made and validated before any device exists.
    let mut shaders = Vec::new();
    for shape in SHAPES {
        for consumer in [Consumer::Exec, Consumer::SavedMask] {
            let mut shader = phase_shader(&mask_program(shape, consumer)).expect("admitted before device creation");
            validate(&mut shader);
            shaders.push((shape, consumer, shader));
        }
        for consumer in [Consumer::Complement, Consumer::Joined] {
            if let Ok(mut shader) = phase_shader(&mask_program(shape, consumer)) {
                validate(&mut shader);
                shaders.push((shape, consumer, shader));
            } else {
                eprintln!("{shape:?} {consumer:?}: refused (fail-closed), not dispatched");
            }
        }
    }
    let _one = super::super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let (host, warp) = explicit_host();
    assert!(!warp, "WARP has no WaveSize32 waves: run on the named hardware adapter only");
    for (shape, consumer, shader) in &shaders {
        for _ in 0..16 {
            dispatch_masks(&host, shader, *consumer);
        }
        eprintln!("{shape:?} {consumer:?}: 16 readbacks PASS");
    }
}
