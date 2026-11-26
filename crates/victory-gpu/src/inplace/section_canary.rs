//! Explicit NVIDIA capability check on owned data; no title or GPU commands.
use super::*;

#[test]
#[ignore = "explicit RTX 5070 Laptop adapter required; device/heap/buffer, no submissions"]
fn named_nvidia_file_mapping_buffer_creation() -> Result<(), String> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter1, IDXGIFactory4};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE as CpuHandle, INVALID_HANDLE_VALUE};

    let wanted = std::env::var("VICTORY_D3D12_ADAPTER").map_err(|_| "explicit adapter required")?;
    if wanted != "NVIDIA GeForce RTX 5070 Laptop GPU" || std::env::var("VICTORY_WARP").is_ok_and(|v| v == "1") {
        return Err("explicit RTX 5070 Laptop hardware name with WARP disabled required".into());
    }
    // This helper honors exact names and checks device/adapter LUID equality.
    // It creates a device only, without a runtime Host singleton or queues.
    let host = victory_shader::emit::host::Host::new(false)?;
    let device = &host.device;
    let error = |e: windows::core::Error| format!("{} ({:?})", e.message(), e.code());
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

    struct Owned { section: CpuHandle, view: mem::MEMORY_MAPPED_VIEW_ADDRESS }
    impl Owned {
        fn finish(&mut self) -> Result<(), String> {
            let mut errors = Vec::new();
            // SAFETY: these are only the fixture's retained handles. On a
            // cleanup error this test fails; process exit reclaims leftovers.
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
            if let Err(error) = self.finish() { eprintln!("owned Stage A cleanup failed: {error}"); }
        }
    }
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
    println!("Stage A adapter={name:?} LUID={:08x}:{:08x}, fileheap={:?}, device={:?}",
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
    let buffer_desc = unsafe { buffer.GetDesc() };
    drop(buffer);
    drop(heap);
    owned.finish()?;
    unsafe { device.GetDeviceRemovedReason().map_err(error)?; }
    println!("Stage A accepted: buffer={buffer_desc:?}, GPUVA={va:#x}, cleanup complete, no application queues/lists/submissions");
    Ok(())
}
