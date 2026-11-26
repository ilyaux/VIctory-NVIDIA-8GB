//! Original allocation-loop canary: iteration order changes which object gets a budget.

use std::collections::HashMap;

use super::{HostShader, lower, validator};
use crate::gfx10::{Encoding, Instruction};
use crate::native::{
    ComputeIo, HwStage, NativeProgram, ResourceType, RootParam, RootType, StageIo, UsedResource,
};
use crate::{Container, ShaderError, io, resource, structure};

fn opcode(encoding: Encoding, name: &str) -> u32 {
    (0..1024)
        .find(|&opcode| Instruction { offset: 0, encoding, opcode, words: Vec::new() }.name() == Some(name))
        .unwrap_or_else(|| panic!("missing opcode {name}")) as u32
}

fn mov(dst: u8, src: u16) -> u32 { 0x7e00_0200 | u32::from(dst) << 17 | u32::from(src) }
fn cmp(name: &str, a: u16, b: u8) -> u32 {
    0x7c00_0000 | opcode(Encoding::Vopc, name) << 17 | u32::from(b) << 9 | u32::from(a)
}
fn vop2(name: &str, dst: u8, a: u16, b: u8) -> u32 {
    opcode(Encoding::Vop2, name) << 25 | u32::from(dst) << 17 | u32::from(b) << 9 | u32::from(a)
}
fn select(dst: u8, no: u16, yes: u16) -> [u32; 2] {
    [0xd400_0000 | opcode(Encoding::Vop3, "v_cndmask_b32") << 16 | u32::from(dst),
        u32::from(no) | u32::from(yes) << 9 | 106 << 18]
}
fn sop1(name: &str, dst: u8, src: u8) -> u32 {
    0xbe80_0000 | u32::from(dst) << 16 | opcode(Encoding::Sop1, name) << 8 | u32::from(src)
}
fn ds(name: &str, offset: u16, dst: u8) -> [u32; 2] {
    let reads = name.starts_with("ds_read") || name.contains("_rtn_");
    [0xd800_0000 | opcode(Encoding::Ds, name) << 18 | u32::from(offset),
        1 | 2 << 8 | if reads { u32::from(dst) << 24 } else { 0 }]
}

#[derive(Default)]
struct Code {
    words: Vec<u32>,
    labels: HashMap<&'static str, usize>,
    branches: Vec<(usize, &'static str)>,
}

impl Code {
    fn put(&mut self, words: impl IntoIterator<Item = u32>) { self.words.extend(words); }
    fn label(&mut self, name: &'static str) { assert!(self.labels.insert(name, self.words.len()).is_none()); }
    fn branch(&mut self, word: u32, name: &'static str) {
        self.branches.push((self.words.len(), name));
        self.words.push(word);
    }
    fn finish(mut self) -> Vec<u8> {
        for (at, name) in self.branches {
            let relative = self.labels[name] as isize - at as isize - 1;
            let offset = i16::try_from(relative).expect("synthetic branch fits i16");
            self.words[at] |= u32::from(offset as u16);
        }
        self.words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }
}

/// Invocation0 requests [0,1]; invocation32 requests [1]. Other invocations do no work.
/// A shared counter grants at most one unit. Correct phase order grants it to invocation32.
fn allocator_program() -> NativeProgram {
    allocator_program_with_oob_probe(false)
}

fn allocator_program_with_oob_probe(probe: bool) -> NativeProgram {
    allocator_variant(probe, None)
}

#[derive(Clone, Copy, PartialEq)]
enum EarlyExit { Direct, Stub, Repeated, RawVccNz, RawVccZ }

fn allocator_variant(probe: bool, early_exit: Option<EarlyExit>) -> NativeProgram {
    let mut c = Code::default();
    c.put([sop1("s_mov_b32", 5, 128), 0x3d << 26 | 0x02 << 18 | 8 << 6 | 2, 125 << 25]);
    // s[8:11] is the raw UAV descriptor; v0 remains the original thread id.
    c.put([mov(1, 128), mov(2, 128), mov(4, 128), mov(6, if early_exit.is_some() { 137 } else { 128 })]);
    if let Some(exit) = early_exit {
        // The raw descriptor and marker9 are already defined. Source EXECNZ takes
        // this exit for the whole wave because invocation0 is active. An independently
        // progressing high half would instead run the allocator and leak marker10 at32.
        c.put([cmp("v_cmp_eq_u32", 128, 0)]);
        if matches!(exit, EarlyExit::RawVccNz | EarlyExit::RawVccZ) {
            c.put([sop1("s_mov_b64", 14, 106), cmp("v_cmp_eq_u32", 160, 0),
                sop1("s_and_saveexec_b64", 18, 14)]);
            // Raw VCC has bit32; current EXEC has only bit0. The scalar branch
            // must test that raw VCC, including bits outside current EXEC.
            if exit == EarlyExit::RawVccNz { c.branch(0xbf87_0000, "store_tail"); }
            else {
                c.branch(0xbf86_0000, "bad_raw_vcc");
                c.branch(0xbf82_0000, "store_tail");
            }
        } else {
            c.put([sop1("s_and_saveexec_b64", 18, 106)]);
            c.branch(0xbf89_0000, match exit {
                EarlyExit::Direct => "store_tail", EarlyExit::Stub => "exit_stub", EarlyExit::Repeated => "repeat_exit",
                _ => unreachable!(),
            });
        }
        c.put([sop1("s_mov_b64", 126, 18)]);
    }
    c.put([cmp("v_cmp_eq_u32", 128, 0), sop1("s_and_saveexec_b64", 16, 106)]);
    c.branch(0xbf88_0000, "init_restore");
    c.put(ds("ds_write_b32", 0, 0));
    c.put(ds("ds_write_b32", 4, 0));
    c.label("init_restore");
    c.put([sop1("s_mov_b64", 126, 16)]);
    c.put([cmp("v_cmp_eq_u32", 160, 0)]); // tid==32
    c.put(select(5, 128, 129)); // limit=1 for invocation32, zero for the others
    c.put([cmp("v_cmp_eq_u32", 128, 0)]);
    c.put(select(5, 261, 130)); // limit=2 for invocation0
    c.put([sop1("s_mov_b64", 12, 126), cmp("v_cmp_gt_u32", 261, 4), sop1("s_and_saveexec_b64", 14, 106)]);
    c.branch(0xbf88_0000, "loop_restore");
    c.label("loop");
    c.put([cmp("v_cmp_eq_u32", 160, 0)]);
    c.put(select(2, 260, 129)); // invocation0 requests its iteration index; invocation32 requests1
    c.put(ds("ds_read_b32", 0, 9)); // disjoint read ensures the mixed-LDS path is exercised
    c.put(ds("ds_add_rtn_u32", 4, 3));
    c.put([cmp("v_cmp_gt_u32", 129, 3)]); // old allocation counter <1
    c.put(select(8, 128, 258));
    c.put([vop2("v_add_nc_u32", 6, 262, 8), vop2("v_add_nc_u32", 4, 129, 4)]);
    c.put([cmp("v_cmp_gt_u32", 261, 4)]);
    c.put([0x8000_0000 | opcode(Encoding::Sop2, "s_and_b64") << 23 | 126 << 16 | 106 << 8 | 126]);
    c.branch(0xbf89_0000, "loop");
    c.label("loop_restore");
    c.put([sop1("s_mov_b64", 126, 12)]);
    c.label("store_tail");
    c.put([vop2("v_lshlrev_b32", 7, 130, 0)]);
    if probe {
        c.put([mov(2, 129)]);
        c.put(ds("ds_write_b32", 512, 0));
        c.put(ds("ds_add_rtn_u32", 512, 10));
        c.put(ds("ds_read_b32", 512, 9));
        c.put([vop2("v_or_b32", 9, 265, 10)]);
        c.put(ds("ds_read_b32", 0, 10));
        c.put([vop2("v_or_b32", 9, 265, 10)]);
    }
    // buffer_store_dword v6, v7(byte offset), s[8:11], 0
    c.put([0x38 << 26 | 0x1c << 18 | 1 << 12, 128 << 24 | 2 << 16 | 6 << 8 | 7]);
    if probe {
        c.put([0x38 << 26 | 0x1c << 18 | 1 << 12 | 256, 128 << 24 | 2 << 16 | 9 << 8 | 7]);
    }
    c.put([0xbf81_0000]);
    if early_exit == Some(EarlyExit::Stub) {
        c.label("exit_stub");
        c.branch(0xbf82_0000, "store_tail");
    }
    if early_exit == Some(EarlyExit::Repeated) {
        c.label("repeat_exit");
        // Before widening the first test this second EXECNZ looks constant-true.
        // After widening it must still be a collective test, or the high half
        // reaches this bad path, restores its lanes and writes marker7.
        c.branch(0xbf89_0000, "store_tail");
        c.put([sop1("s_mov_b64", 126, 18), mov(6, 135)]);
        c.branch(0xbf82_0000, "store_tail");
    }
    if early_exit == Some(EarlyExit::RawVccZ) {
        c.label("bad_raw_vcc");
        c.put([sop1("s_mov_b64", 126, 18), mov(6, 135)]);
        c.branch(0xbf82_0000, "store_tail");
    }
    NativeProgram {
        stage: HwStage::Cs, wave64: true, vgprs: 16,
        layout: vec![
            RootParam { root: RootType::Constants, table_offset_dwords: 0, sgprs: 4, base_register: 0, user_data: 0, slot: 0 },
            RootParam { root: RootType::Table, table_offset_dwords: 0, sgprs: 1, base_register: 0, user_data: 4, slot: 1 },
        ],
        used: vec![UsedResource { kind: ResourceType::Uav, root: RootType::Table, table_offset_dwords: 0, lower_bound: 0, stride: 0, dwords: 4, user_data: 4, slot: 1 }],
        io: StageIo::Cs(ComputeIo { rsrc2: 5 << 1 | 1 << 15, threads: [64, 1, 1] }),
        code: c.finish(),
    }
}

fn empty_container() -> Container<'static> { Container { digest: [0; 16], version: (1, 0), parts: Vec::new() } }

fn phase_shader(native: &NativeProgram) -> Result<HostShader, ShaderError> {
    let p = crate::spill::build(native)?;
    let previous = lower::TEST_HOST_WAVE.replace(Some(32));
    let result = super::try_lds_phases(&empty_container(), native, &p);
    lower::TEST_HOST_WAVE.set(previous);
    let (shader, wave) = result?;
    assert_eq!(wave, Some(32), "a strict phase candidate must explicitly require WaveSize32");
    Ok(shader)
}

fn baseline_shader(native: &NativeProgram, host_wave: u32) -> (HostShader, Option<u32>) {
    let p = crate::spill::build(native).unwrap();
    let st = structure::structure(&p).unwrap();
    let res = resource::track(&p, &native.layout, native.stage, Some(&st)).unwrap();
    let io = io::raise(&p, native).unwrap();
    let previous = lower::TEST_HOST_WAVE.replace(Some(host_wave));
    let result = super::emit_program(&empty_container(), native, &p, &st, &io, &res);
    lower::TEST_HOST_WAVE.set(previous);
    result.unwrap()
}

fn validate(shader: &mut HostShader) {
    let mut validator = validator::Validator::load(validator::Validator::default_path().as_deref()).expect("CPU DXIL validator");
    validator.validate(&mut shader.dxil).expect("synthetic DXIL validation");
}

#[test]
fn equal_totals_do_not_prove_equal_object_allocations() {
    let grant = |order: &[(usize, u32)]| {
        let mut counter = 0u32;
        let mut accepted = [0u32; 64];
        for &(lane, iteration) in order {
            let wanted = if lane == 0 { iteration } else { 1 };
            accepted[lane] += wanted.min(1u32.saturating_sub(counter));
            counter += wanted;
        }
        accepted
    };
    let phased = grant(&[(0, 0), (32, 0), (0, 1)]);
    let independent_halves = grant(&[(0, 0), (0, 1), (32, 0)]);
    assert_eq!(phased.iter().sum::<u32>(), 1);
    assert_eq!(independent_halves.iter().sum::<u32>(), 1);
    assert_eq!(phased[32], 1);
    assert_eq!(independent_halves[0], 1);
    assert_ne!(phased, independent_halves);
}

#[test]
fn allocator_loop_requires_phases_and_emits_validated_wave32() {
    let native = allocator_program();
    assert_eq!(baseline_shader(&native, 32).1, Some(64), "the canary must cover a previously refused mixed region");
    let mut shader = phase_shader(&native).expect("strict phase lowering");
    validate(&mut shader);
}

#[test]
fn native_lds_bounds_are_preserved_with_exchange_storage() {
    let native = allocator_program_with_oob_probe(true);
    let mut shader = phase_shader(&native).expect("strict bounded LDS lowering");
    validate(&mut shader);
}

#[test]
fn multiwave_groups_and_plain_jump_backedges_are_refused() {
    let mut native = allocator_program();
    if let StageIo::Cs(cs) = &mut native.io { cs.threads = [128, 1, 1]; }
    assert!(phase_shader(&native).is_err());
    let mut native = allocator_program();
    let branch = native.code.chunks_exact(4).position(|word| u32::from_le_bytes(word.try_into().unwrap()) >> 16 == 0xbf89).unwrap();
    let original = u32::from_le_bytes(native.code[branch * 4..branch * 4 + 4].try_into().unwrap());
    native.code[branch * 4..branch * 4 + 4].copy_from_slice(&(0xbf82_0000 | original & 0xffff).to_le_bytes());
    assert!(phase_shader(&native).is_err());
}

#[test]
fn a_divergent_early_end_is_refused() {
    let mut native = allocator_program();
    let count = native.code.len() / 4;
    let jump = u16::try_from(count + 1).unwrap();
    let mut code: Vec<u8> = [cmp("v_cmp_eq_u32", 128, 0), sop1("s_and_saveexec_b64", 18, 106), 0xbf89_0000 | u32::from(jump), sop1("s_mov_b64", 126, 18)]
        .iter().flat_map(|word| word.to_le_bytes()).collect();
    code.extend(native.code);
    code.extend(0xbf81_0000u32.to_le_bytes());
    native.code = code;
    assert!(phase_shader(&native).is_err());
}

#[test]
fn raw_vcc_branches_include_bits_outside_current_exec() {
    for exit in [EarlyExit::RawVccNz, EarlyExit::RawVccZ] {
        let native = allocator_variant(false, Some(exit));
        let p = crate::spill::build(&native).unwrap();
        for branch in p.blocks.iter().flat_map(|b| &b.insts).filter(|i|
            matches!(i.name(), "s_cbranch_vccz" | "s_cbranch_vccnz")) {
            assert!(branch.read(crate::ir::Reg::S(crate::ir::EXEC)).is_none(),
                "raw VCC tests must not acquire an implicit EXEC dependency");
        }
        let mut shader = phase_shader(&native).expect("raw VCC branch");
        validate(&mut shader);
    }
}

#[test]
fn a_source_wave_exit_is_collective_even_without_an_exec_restore() {
    for exit in [EarlyExit::Direct, EarlyExit::Stub, EarlyExit::Repeated] {
        let mut shader = phase_shader(&allocator_variant(false, Some(exit)))
            .expect("source group early exit, including a forwarding block");
        validate(&mut shader);
    }
}

#[test]
#[ignore = "manual CPU-only real case: VICTORY_PHASE_CASE, no GPU"]
fn external_case_requires_wave32_and_cpu_validation() {
    let bytes = std::fs::read(std::env::var_os("VICTORY_PHASE_CASE").expect("explicit external container path")).unwrap();
    let c = Container::parse(&bytes).unwrap();
    let part = c.part(crate::FourCC::DXIL).or(c.part(crate::FourCC::ACCC)).unwrap();
    let native = crate::native_programs(part.data).unwrap().into_iter().find(|n| n.stage == HwStage::Cs).unwrap();
    let p = crate::spill::build(&native).unwrap();
    let previous = lower::TEST_HOST_WAVE.replace(Some(32));
    let result = super::try_lds_phases(&c, &native, &p);
    let (mut shader, wave) = result.expect("strict real-case phase candidate");
    assert_eq!(wave, Some(32));
    validate(&mut shader);
    eprintln!("external case: WaveSize32, CPU validator PASS, {} bytes", shader.dxil.len());
    if std::env::var("VICTORY_LDS_PHASES").as_deref() == Ok("1") {
        let st = structure::structure(&p).unwrap();
        let io = io::raise(&p, &native).unwrap();
        let res = resource::track(&p, &native.layout, native.stage, Some(&st)).unwrap();
        let mut integrated = super::emit(&c, &native, &p, &st, &io, &res).unwrap();
        validate(&mut integrated);
        assert_eq!(shader.dxil, integrated.dxil, "the public emitter must select the audited candidate");
        assert!(env!("VICTORY_SHADER_SWITCHES").split(',').any(|n| n == "VICTORY_LDS_PHASES"));
        let setting = b"VICTORY_LDS_PHASES=1\0";
        assert!(super::translation_fingerprint().windows(setting.len()).any(|part| part == setting));
        eprintln!("public emitter: strict phase candidate selected, cache switch present");
    }
    lower::TEST_HOST_WAVE.set(previous);
}

fn explicit_host() -> (super::host::Host, bool) {
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
    use windows::Win32::Graphics::Direct3D12::{D3D12CreateDevice, D3D12GetDebugInterface,
        D3D12_FEATURE_D3D12_OPTIONS1, D3D12_FEATURE_DATA_D3D12_OPTIONS1, ID3D12Debug, ID3D12Device};
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter1, IDXGIFactory4};

    let warp = std::env::var("VICTORY_D3D12_WARP").is_ok_and(|v| v == "1");
    let name = std::env::var("VICTORY_D3D12_ADAPTER").ok().filter(|v| !v.is_empty());
    assert_ne!(warp, name.is_some(), "select exactly one explicit WARP or named hardware adapter");
    // SAFETY: COM out pointers and adapter/device lifetimes stay valid; no default adapter is used.
    unsafe {
        if std::env::var_os("VICTORY_D3D12_DEBUG").is_some() {
            let mut debug: Option<ID3D12Debug> = None;
            D3D12GetDebugInterface(&mut debug).expect("requested D3D12 debug layer");
            debug.unwrap().EnableDebugLayer();
        }
        let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).unwrap();
        let adapter: IDXGIAdapter1 = if warp {
            factory.EnumWarpAdapter().unwrap()
        } else {
            let mut found = Vec::new();
            let mut index = 0;
            while let Ok(adapter) = factory.EnumAdapters1(index) {
                let desc = adapter.GetDesc1().unwrap();
                let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
                let label = String::from_utf16_lossy(&desc.Description[..end]);
                if Some(&label) == name.as_ref() { found.push(adapter); }
                index += 1;
            }
            assert_eq!(found.len(), 1, "named hardware adapter must be present and unambiguous");
            found.pop().unwrap()
        };
        let desc = adapter.GetDesc1().unwrap();
        assert_eq!(desc.Flags & 2 != 0, warp, "software/hardware adapter selection");
        let mut device: Option<ID3D12Device> = None;
        D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_12_0, &mut device).unwrap();
        let device = device.unwrap();
        let actual: IDXGIAdapter1 = factory.EnumAdapterByLuid(device.GetAdapterLuid()).unwrap();
        let actual = actual.GetDesc1().unwrap();
        let end = actual.Description.iter().position(|&c| c == 0).unwrap_or(actual.Description.len());
        let label = String::from_utf16_lossy(&actual.Description[..end]);
        assert_eq!(actual.Flags & 2 != 0, warp);
        if let Some(name) = name { assert_eq!(label, name); }
        eprintln!("LDS phase GPU canary device: {label}; software={warp}; vendor={:#x}", actual.VendorId);
        let mut caps = D3D12_FEATURE_DATA_D3D12_OPTIONS1::default();
        device.CheckFeatureSupport(D3D12_FEATURE_D3D12_OPTIONS1, (&raw mut caps).cast(), size_of_val(&caps) as u32).unwrap();
        eprintln!("device wave operations={} lane range={}..{}", caps.WaveOps.as_bool(), caps.WaveLaneCountMin, caps.WaveLaneCountMax);
        assert!(caps.WaveOps.as_bool() && caps.WaveLaneCountMin <= 32 && caps.WaveLaneCountMax >= 32,
            "selected device cannot execute the actual WaveSize32 candidate; do not change shader metadata to bypass this");
        (super::host::Host { device }, warp)
    }
}

fn dispatch_allocator(host: &super::host::Host, shader: &HostShader, probe: bool, early_exit: bool) {
    use super::host::{Dispatch, Root, View};
    let (root, pipeline) = host.compute_pipeline(shader).expect("explicit device pipeline");
    let out = host.dispatch(&root, &pipeline, &Dispatch {
        roots: vec![(0, Root::Constants(vec![0; shader.user_data_dwords().max(1) as usize]))],
        views: &[View::RawUav(if probe { 128 } else { 64 })], samplers: &[], groups: [1, 1, 1],
    }).expect("bounded offscreen allocation dispatch");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].len(), if probe { 128 } else { 64 });
    let expected: Vec<_> = (0..if probe { 128 } else { 64 })
        .map(|i| if early_exit { if i == 0 { 9 } else { 0 } } else { u32::from(i == 32) }).collect();
    assert_eq!(out[0], expected, "check accepted work per invocation, not only the total counter");
}

#[test]
#[ignore = "manual: diagnostics, loop limit16, explicit WARP/named device and VICTORY_LDS_PHASE_GPU_TESTS=1"]
fn allocator_phase_manual_preserves_objects_on_explicit_device() {
    assert_eq!(std::env::var("VICTORY_LDS_PHASE_GPU_TESTS").as_deref(), Ok("1"));
    assert!(cfg!(feature = "diagnostics"), "a bounded diagnostics canary build is required");
    assert_eq!(super::loop_limit(), 16, "set VICTORY_SHADER_LOOP_LIMIT=16 before this process starts");
    let native = allocator_program();
    let mut phase = phase_shader(&native).expect("strict phases before device creation");
    validate(&mut phase);
    let mut bounded = phase_shader(&allocator_program_with_oob_probe(true)).expect("native bounds before device creation");
    validate(&mut bounded);
    let mut exits: Vec<_> = [EarlyExit::Direct, EarlyExit::Stub, EarlyExit::Repeated, EarlyExit::RawVccNz, EarlyExit::RawVccZ].into_iter().map(|exit|
        phase_shader(&allocator_variant(false, Some(exit))).expect("collective exit before device creation")).collect();
    for shader in &mut exits { validate(shader); }
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let (host, warp) = explicit_host();
    if warp {
        let (mut control, wave) = baseline_shader(&native, 64);
        assert_eq!(wave, Some(64));
        validate(&mut control);
        eprintln!("creating WARP Wave64 reference pipeline");
        dispatch_allocator(&host, &control, false, false);
        eprintln!("WARP Wave64 reference: per-invocation allocation PASS");
    }
    eprintln!("creating Wave32 strict phase pipeline");
    for _ in 0..16 { dispatch_allocator(&host, &phase, false, false); }
    for _ in 0..16 { dispatch_allocator(&host, &bounded, true, false); }
    for shader in &exits { for _ in 0..8 { dispatch_allocator(&host, shader, false, true); } }
    eprintln!("Wave32 strict phases: 16 allocation +16 native-bounds +40 collective-exit/raw-VCC completed readbacks PASS");
}
