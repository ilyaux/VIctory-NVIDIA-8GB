//! Explicit NVIDIA checks on owned data; never opens title memory.
use super::*;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE as CpuHandle, INVALID_HANDLE_VALUE};

struct Owned { section: CpuHandle, view: mem::MEMORY_MAPPED_VIEW_ADDRESS }
impl Owned {
    fn finish(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        // SAFETY: only the fixture's handles, after all GPU use completes.
        unsafe {
            if !self.view.Value.is_null() {
                if mem::UnmapViewOfFile(self.view) == 0 { errors.push(std::io::Error::last_os_error().to_string()); }
                self.view.Value = std::ptr::null_mut();
            }
            if !self.section.is_null() {
                if CloseHandle(self.section) == 0 { errors.push(std::io::Error::last_os_error().to_string()); }
                self.section = std::ptr::null_mut();
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if let Err(error) = self.finish() { eprintln!("owned section cleanup failed: {error}"); }
    }
}

// Declaration order releases buffer/heap before their CPU backing.
struct SectionBuffer {
    buffer: ID3D12Resource,
    heap: ID3D12Heap,
    owned: Owned,
    host: victory_shader::emit::host::Host,
}
impl SectionBuffer {
    fn finish(self) -> Result<(), String> {
        let Self { buffer, heap, mut owned, host } = self;
        drop(buffer);
        drop(heap);
        owned.finish()?;
        unsafe { host.device.GetDeviceRemovedReason().map_err(error) }
    }
}
fn error(e: windows::core::Error) -> String { format!("{} ({:?})", e.message(), e.code()) }

fn section_buffer() -> Result<SectionBuffer, String> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter1, IDXGIFactory4};

    let wanted = std::env::var("VICTORY_D3D12_ADAPTER").map_err(|_| "explicit adapter required")?;
    if wanted != "NVIDIA GeForce RTX 5070 Laptop GPU" || std::env::var("VICTORY_WARP").is_ok_and(|v| v == "1") {
        return Err("explicit RTX 5070 Laptop hardware name with WARP disabled required".into());
    }
    // This helper honors exact names and checks device/adapter LUID equality.
    // It creates a device only, without a runtime Host singleton or queues.
    let host = victory_shader::emit::host::Host::new(false)?;
    let device = &host.device;
    // SAFETY: plain device/adapter metadata queries, no command submission.
    let (name, luid) = unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(error)?;
        let luid = device.GetAdapterLuid();
        let adapter: IDXGIAdapter1 = factory.EnumAdapterByLuid(luid).map_err(error)?;
        let desc = adapter.GetDesc1().map_err(error)?;
        let end = desc.Description.iter().position(|&v| v == 0).unwrap_or(desc.Description.len());
        let name = String::from_utf16(&desc.Description[..end]).map_err(|e| e.to_string())?;
        if name != wanted || desc.VendorId != 0x10de || desc.Flags & 2 != 0 {
            return Err("device is not the requested NVIDIA hardware adapter".into());
        }
        device.cast::<ID3D12Device3>().map_err(error)?;
        (name, luid)
    };

    // SAFETY: one fresh, unnamed pagefile section of 64 KiB, no game data.
    let section = unsafe { mem::CreateFileMappingW(INVALID_HANDLE_VALUE, std::ptr::null(),
        mem::PAGE_READWRITE | mem::SEC_COMMIT, 0, K64 as u32, std::ptr::null()) };
    if section.is_null() { return Err(std::io::Error::last_os_error().to_string()); }
    let mut owned = Owned { section, view: mem::MEMORY_MAPPED_VIEW_ADDRESS { Value: std::ptr::null_mut() } };
    owned.view = unsafe { mem::MapViewOfFile(section, mem::FILE_MAP_WRITE, 0, 0, K64) };
    if owned.view.Value.is_null() { return Err(std::io::Error::last_os_error().to_string()); }
    let pattern = [17u32, 25, 0xc0ff_ee];
    // SAFETY: a writable committed view exclusively owned by this fixture.
    unsafe { std::ptr::copy_nonoverlapping(pattern.as_ptr(), owned.view.Value.cast::<u32>(), pattern.len()); }
    let heap_result = open(device, From::Section(section as usize));
    println!("Owned section adapter={name:?} LUID={:08x}:{:08x}, fileheap={:?}, device={:?}",
        luid.HighPart as u32, luid.LowPart, heap_result.as_ref().map(|_| ()).map_err(|e| e.code()),
        unsafe { device.GetDeviceRemovedReason() });
    unsafe { device.GetDeviceRemovedReason().map_err(error)?; }
    let heap = heap_result.map_err(error)?;
    let desc = unsafe { heap.GetDesc() };
    println!("owned section bytes={K64}; heap={desc:?}");
    if desc.SizeInBytes < K64 as u64
        || desc.Properties.Type != D3D12_HEAP_TYPE_CUSTOM
        || desc.Properties.CPUPageProperty != D3D12_CPU_PAGE_PROPERTY_WRITE_BACK
        || desc.Properties.MemoryPoolPreference != D3D12_MEMORY_POOL_L0
        || desc.Flags.0 & D3D12_HEAP_FLAG_SHARED_CROSS_ADAPTER.0 == 0 {
        return Err("unexpected file-mapping heap properties; no fallback attempted".into());
    }
    let resource_desc = buffer_desc(K64 as u64,
        D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS);
    let allocation = unsafe { device.GetResourceAllocationInfo(0, &[resource_desc]) };
    if allocation.SizeInBytes > desc.SizeInBytes || allocation.Alignment > desc.Alignment {
        return Err(format!("buffer allocation does not fit owned heap: {allocation:?}"));
    }
    let buffer_result = place(device, &heap, 0, K64 as u64);
    println!("placed buffer={:?}, device={:?}", buffer_result.as_ref().map(|_| ()).map_err(|e| e.code()),
        unsafe { device.GetDeviceRemovedReason() });
    let buffer = buffer_result.map_err(error)?;
    unsafe { device.GetDeviceRemovedReason().map_err(error)?; }
    let va = unsafe { buffer.GetGPUVirtualAddress() };
    if va == 0 { return Err("placed buffer has no GPU virtual address".into()); }
    if unsafe { std::slice::from_raw_parts(owned.view.Value.cast::<u32>(), pattern.len()) } != pattern {
        return Err("CPU-owned pattern changed without a submission".into());
    }
    unsafe { device.GetDeviceRemovedReason().map_err(error)?; }
    println!("buffer={:?}, GPUVA={va:#x}", unsafe { buffer.GetDesc() });
    Ok(SectionBuffer { buffer, heap, owned, host })
}

#[test]
#[ignore = "explicit RTX 5070 Laptop adapter required; device/heap/buffer, no submissions"]
fn named_nvidia_file_mapping_buffer_creation() -> Result<(), String> {
    section_buffer()?.finish()?;
    println!("Stage A accepted: cleanup complete, no application queues/lists/submissions");
    Ok(())
}

#[test]
#[ignore = "explicit RTX 5070 Laptop; one owned 1-thread GPU dispatch; run alone in its own test process"]
fn named_nvidia_file_mapping_buffer_coherence() -> Result<(), String> {
    struct Resources {
        back: ID3D12Resource,
        root: ID3D12RootSignature,
        pso: ID3D12PipelineState,
        rig: Rig,
        section: SectionBuffer,
    }
    struct Flight { resources: Option<Resources>, submitted: bool, completed: bool }
    impl Drop for Flight {
        fn drop(&mut self) {
            if self.submitted && !self.completed {
                // A timeout is invalid evidence, not permission to free bytes
                // the GPU may use. The dedicated test process reclaims these.
                if let Some(resources) = self.resources.take() { std::mem::forget(resources); }
                eprintln!("Stage B INVALID: retained in-flight resources until process exit; no retry");
            }
        }
    }
    let section = section_buffer()?;
    let device = &section.host.device;
    let rig = Rig::new(device).map_err(error)?;
    let back = rig.committed(D3D12_HEAP_TYPE_READBACK, K64 as u64, D3D12_RESOURCE_STATE_COPY_DEST).map_err(error)?;
    const SOURCE: &str = "#define RS \"UAV(u0)\"\nRWByteAddressBuffer b : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { b.Store(8, b.Load(0) + b.Load(4)); }\n";
    // SAFETY: self-authored source, blobs and device live through creation.
    let (root, pso): (ID3D12RootSignature, ID3D12PipelineState) = unsafe {
        let mut code: Option<ID3DBlob> = None;
        D3DCompile(SOURCE.as_ptr().cast(), SOURCE.len(), s!("owned-section-canary"), None, None,
            s!("main"), s!("cs_5_1"), 0, 0, &mut code, None).map_err(error)?;
        let code = code.ok_or("compiler returned no code")?;
        let bytes = std::slice::from_raw_parts(code.GetBufferPointer().cast::<u8>(), code.GetBufferSize());
        let root: ID3D12RootSignature = device.CreateRootSignature(0, bytes).map_err(error)?;
        let pso = device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
            // Borrowed root; the descriptor's ManuallyDrop never releases it.
            pRootSignature: std::mem::transmute_copy(&root),
            CS: D3D12_SHADER_BYTECODE { pShaderBytecode: bytes.as_ptr().cast(), BytecodeLength: bytes.len() },
            ..Default::default()
        }).map_err(error)?;
        (root, pso)
    };
    // Seed all 64 KiB so every byte outside the one result word is checked.
    let mut expected = vec![0xa5a5_5a5au32; K64 / 4];
    expected[..3].copy_from_slice(&[17, 25, 0xc0ff_ee]);
    unsafe { std::ptr::copy_nonoverlapping(expected.as_ptr(), section.owned.view.Value.cast::<u32>(), expected.len()); }
    expected[2] = 42;
    let mut flight = Flight { resources: Some(Resources { back, root, pso, rig, section }), submitted: false, completed: false };
    let r = flight.resources.as_ref().unwrap();
    // SAFETY: all addresses belong to the retained fixture. No tile/texture/
    // reserved resources or title memory; CPU stops touching it until fence 1.
    unsafe {
        r.rig.allocator.Reset().map_err(error)?;
        r.rig.list.Reset(&r.rig.allocator, None).map_err(error)?;
        r.rig.list.ResourceBarrier(&[crate::host::transition(&r.section.buffer,
            D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_UNORDERED_ACCESS)]);
        r.rig.list.SetComputeRootSignature(&r.root);
        r.rig.list.SetPipelineState(&r.pso);
        r.rig.list.SetComputeRootUnorderedAccessView(0, r.section.buffer.GetGPUVirtualAddress());
        r.rig.list.Dispatch(1, 1, 1);
        r.rig.list.ResourceBarrier(&[D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
                    pResource: std::mem::transmute_copy(&r.section.buffer),
                }),
            },
            ..Default::default()
        }]);
        r.rig.list.ResourceBarrier(&[crate::host::transition(&r.section.buffer,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_COPY_SOURCE)]);
        r.rig.list.CopyBufferRegion(&r.back, 0, &r.section.buffer, 0, K64 as u64);
        r.rig.list.Close().map_err(error)?;
        let list: ID3D12CommandList = r.rig.list.cast().map_err(error)?;
        flight.submitted = true;
        r.rig.queue.ExecuteCommandLists(&[Some(list)]);
        r.rig.queue.Signal(&r.rig.fence, 1).map_err(error)?;
        r.rig.fence.SetEventOnCompletion(1, r.rig.event).map_err(error)?;
        let wait = WaitForSingleObject(r.rig.event, 5_000);
        let done = r.rig.fence.GetCompletedValue();
        if wait.0 != 0 || done != 1 {
            return Err(format!("fence invalid: wait={wait:?}, completed={done}; device={:?}", r.rig.device.GetDeviceRemovedReason()));
        }
        r.rig.device.GetDeviceRemovedReason().map_err(error)?;
        flight.completed = true;
        let mut data = std::ptr::null_mut();
        r.back.Map(0, None, Some(&mut data)).map_err(error)?;
        let readback_ok = std::slice::from_raw_parts(data.cast::<u32>(), expected.len()) == expected;
        r.back.Unmap(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }));
        let cpu_ok = std::slice::from_raw_parts(r.section.owned.view.Value.cast::<u32>(), expected.len()) == expected;
        if !readback_ok || !cpu_ok { return Err(format!("coherence mismatch: readback={readback_ok}, CPU alias={cpu_ok}")); }
    }
    let Resources { back, root, pso, rig, section } = flight.resources.take().unwrap();
    drop(back);
    drop(pso);
    drop(root);
    drop(rig);
    section.finish()?;
    println!("Stage B accepted: one group/thread, 17+25=42 via CPU alias and GPU readback, all 64KiB sentinels intact, fence=1, cleanup complete");
    Ok(())
}
