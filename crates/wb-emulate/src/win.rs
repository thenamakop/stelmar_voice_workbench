//! Windows and Vulkan calls for the emulator: processor topology, the Job Object, memory commit
//! probing and Vulkan device enumeration.
//!
//! The only module in `wb-emulate` that may use `unsafe` (docs/UNSAFE.md). Each block holds one
//! call and says why it is sound. Parsing of returned buffers happens in safe code.
#![allow(unsafe_code)]

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT, JOB_OBJECT_LIMIT_AFFINITY,
    JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
};
use windows::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, RelationProcessorCore,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};
use windows::core::PCWSTR;

/// Raw `RelationProcessorCore` records from GetLogicalProcessorInformationEx.
pub fn processor_core_records() -> Result<Vec<u8>> {
    let mut len = 0u32;
    // SAFETY: a null buffer with a length out-pointer asks for the size needed; the call fails
    // with ERROR_INSUFFICIENT_BUFFER, which is expected here.
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut len) };
    if len == 0 {
        bail!("GetLogicalProcessorInformationEx returned no size");
    }
    // u64 storage keeps the records 8-byte aligned, as the structures require.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    let ptr = buf
        .as_mut_ptr()
        .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
    // SAFETY: `buf` holds at least `len` writable, aligned bytes, and `len` says so.
    unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, Some(ptr), &mut len) }
        .context("GetLogicalProcessorInformationEx")?;
    let bytes: Vec<u8> = buf
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .take(len as usize)
        .collect();
    Ok(bytes)
}

/// This process's affinity mask.
pub fn process_affinity() -> Result<usize> {
    let mut process_mask = 0usize;
    let mut system_mask = 0usize;
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no cleanup.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `process` is valid and both out-pointers point to live usize values.
    unsafe { GetProcessAffinityMask(process, &mut process_mask, &mut system_mask) }
        .context("GetProcessAffinityMask")?;
    Ok(process_mask)
}

/// A Job Object holding this process (and every child it starts). Closing it kills the job.
pub struct Job(HANDLE);

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: self.0 is the job handle CreateJobObjectW returned; closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Creates a job with an affinity mask and a job-wide commit limit (either optional) and puts
/// this process in it.
pub fn enter_job(affinity: Option<usize>, job_memory: Option<usize>) -> Result<Job> {
    // SAFETY: no security attributes and no name: an anonymous job owned by this process.
    let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.context("CreateJobObjectW")?;
    let job = Job(handle);
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.0;
    if let Some(mask) = affinity {
        flags |= JOB_OBJECT_LIMIT_AFFINITY.0;
        info.BasicLimitInformation.Affinity = mask;
    }
    if let Some(bytes) = job_memory {
        flags |= JOB_OBJECT_LIMIT_JOB_MEMORY.0;
        info.JobMemoryLimit = bytes;
    }
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT(flags);
    let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
    // SAFETY: `info` is a fully initialised JOBOBJECT_EXTENDED_LIMIT_INFORMATION of `size` bytes
    // that lives for the call; the class matches the structure.
    unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&info).cast(),
            size,
        )
    }
    .context("SetInformationJobObject")?;
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no cleanup.
    let me = unsafe { GetCurrentProcess() };
    // SAFETY: both handles are valid; a process may join a job (nested jobs are allowed on
    // Windows 8 and later).
    unsafe { AssignProcessToJobObject(job.0, me) }.context("AssignProcessToJobObject")?;
    Ok(job)
}

/// Commits memory in `step` chunks until the commit fails or `max` is reached, then releases it.
/// Returns the bytes committed.
pub fn probe_commit(step: usize, max: usize) -> usize {
    let mut blocks = Vec::new();
    let mut total = 0usize;
    while total + step <= max {
        // SAFETY: a fresh reservation and commit of `step` bytes at an address the system picks.
        let p = unsafe { VirtualAlloc(None, step, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        if p.is_null() {
            break;
        }
        blocks.push(p);
        total += step;
    }
    for p in blocks {
        // SAFETY: `p` came from VirtualAlloc above and is released once, whole (size 0).
        let _ = unsafe { VirtualFree(p, 0, MEM_RELEASE) };
    }
    total
}

/// One Vulkan physical device, in the loader's enumeration order (the order
/// GGML_VK_VISIBLE_DEVICES indexes).
#[derive(Debug, Clone)]
pub struct VkDevice {
    pub index: usize,
    pub name: String,
    pub vendor_id: u32,
    pub integrated: bool,
    /// Adapter LUID (hex), the same value DXGI reports; equal LUIDs mean one GPU listed twice.
    pub luid: Option<String>,
    /// Driver name and version string, for example "Intel Corporation 101.7092".
    pub driver: String,
}

/// Lists Vulkan physical devices through the system's vulkan-1.dll.
pub fn vulkan_devices() -> Result<Vec<VkDevice>> {
    use ash::vk;
    // SAFETY: loads vulkan-1.dll from the system; the Entry keeps it loaded while used.
    let entry =
        unsafe { ash::Entry::load() }.map_err(|e| anyhow!("Vulkan loader not found: {e}"))?;
    let app = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 1, 0));
    let create = vk::InstanceCreateInfo::default().application_info(&app);
    // SAFETY: `create` and `app` are valid for the call; no allocator.
    let instance = unsafe { entry.create_instance(&create, None) }.context("vkCreateInstance")?;
    // SAFETY: `instance` is live.
    let physical = unsafe { instance.enumerate_physical_devices() };
    let mut out = Vec::new();
    if let Ok(list) = &physical {
        for (index, pd) in list.iter().enumerate() {
            let mut id_props = vk::PhysicalDeviceIDProperties::default();
            let mut driver_props = vk::PhysicalDeviceDriverProperties::default();
            let mut props2 = vk::PhysicalDeviceProperties2::default()
                .push_next(&mut id_props)
                .push_next(&mut driver_props);
            // SAFETY: `pd` was enumerated from this live Vulkan 1.1 instance and `props2` is a
            // valid structure chain that lives for the call.
            unsafe { instance.get_physical_device_properties2(*pd, &mut props2) };
            let props = props2.properties;
            let name = props
                .device_name_as_c_str()
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or_default();
            let luid = (id_props.device_luid_valid == vk::TRUE).then(|| {
                id_props
                    .device_luid
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            });
            let text = |r: Result<&std::ffi::CStr, std::ffi::FromBytesUntilNulError>| {
                r.map(|c| c.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            let driver = format!(
                "{} {}",
                text(driver_props.driver_name_as_c_str()),
                text(driver_props.driver_info_as_c_str())
            );
            out.push(VkDevice {
                index,
                name,
                vendor_id: props.vendor_id,
                integrated: props.device_type == vk::PhysicalDeviceType::INTEGRATED_GPU,
                luid,
                driver,
            });
        }
    }
    // SAFETY: no object created from the instance outlives it; destroyed once, no allocator.
    unsafe { instance.destroy_instance(None) };
    physical.context("vkEnumeratePhysicalDevices")?;
    Ok(out)
}

/// What CUDA sees from this process: whether nvcuda.dll loaded, cuInit's result and the device
/// count. With CUDA_VISIBLE_DEVICES=-1 the count must be 0.
#[derive(Debug, Clone)]
pub struct CudaProbe {
    pub loaded: bool,
    pub init_result: i32,
    pub devices: i32,
    /// A context was created on device 0 and held (the positive control for nvidia-smi).
    pub context_held: bool,
}

type CuInit = unsafe extern "system" fn(u32) -> i32;
type CuDeviceGetCount = unsafe extern "system" fn(*mut i32) -> i32;
type CuDeviceGet = unsafe extern "system" fn(*mut i32, i32) -> i32;
type CuCtxCreate = unsafe extern "system" fn(*mut *mut core::ffi::c_void, u32, i32) -> i32;
type CuCtxDestroy = unsafe extern "system" fn(*mut core::ffi::c_void) -> i32;

/// Loads nvcuda.dll (the NVIDIA driver's CUDA library), counts devices and, with `hold`, keeps a
/// context on device 0 for that long so nvidia-smi lists this process.
pub fn cuda_probe(hold: Option<std::time::Duration>) -> CudaProbe {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{HSTRING, PCSTR};
    let mut out = CudaProbe {
        loaded: false,
        init_result: -1,
        devices: 0,
        context_held: false,
    };
    // SAFETY: loads a system library by name; the handle is kept for the life of the process.
    let Ok(lib) = (unsafe { LoadLibraryW(&HSTRING::from("nvcuda.dll")) }) else {
        return out;
    };
    out.loaded = true;
    // SAFETY: `lib` is a loaded module and the name is a NUL-terminated literal.
    let init = unsafe { GetProcAddress(lib, PCSTR(c"cuInit".as_ptr().cast())) };
    // SAFETY: as above.
    let count = unsafe { GetProcAddress(lib, PCSTR(c"cuDeviceGetCount".as_ptr().cast())) };
    let (Some(init), Some(count)) = (init, count) else {
        return out;
    };
    // SAFETY: cuInit has the CUDA driver API signature `CUresult cuInit(unsigned int)`.
    let init: CuInit =
        unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, CuInit>(init) };
    // SAFETY: cuDeviceGetCount has the signature `CUresult cuDeviceGetCount(int*)`.
    let count: CuDeviceGetCount = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, CuDeviceGetCount>(count)
    };
    // SAFETY: cuInit takes flags by value (0 is required).
    out.init_result = unsafe { init(0) };
    if out.init_result != 0 {
        return out;
    }
    let mut n = 0i32;
    // SAFETY: `n` is a valid out-pointer for the count.
    if unsafe { count(&mut n) } == 0 {
        out.devices = n;
    }
    let Some(hold) = hold.filter(|_| out.devices > 0) else {
        return out;
    };
    // SAFETY: `lib` is loaded and the names are NUL-terminated literals.
    let get = unsafe { GetProcAddress(lib, PCSTR(c"cuDeviceGet".as_ptr().cast())) };
    // SAFETY: as above.
    let create = unsafe { GetProcAddress(lib, PCSTR(c"cuCtxCreate_v2".as_ptr().cast())) };
    // SAFETY: as above.
    let destroy = unsafe { GetProcAddress(lib, PCSTR(c"cuCtxDestroy_v2".as_ptr().cast())) };
    let (Some(get), Some(create), Some(destroy)) = (get, create, destroy) else {
        return out;
    };
    // SAFETY: cuDeviceGet has the signature `CUresult cuDeviceGet(CUdevice*, int)`.
    let get: CuDeviceGet =
        unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, CuDeviceGet>(get) };
    // SAFETY: cuCtxCreate_v2 has the signature `CUresult cuCtxCreate(CUcontext*, unsigned, CUdevice)`.
    let create: CuCtxCreate =
        unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, CuCtxCreate>(create) };
    // SAFETY: cuCtxDestroy_v2 has the signature `CUresult cuCtxDestroy(CUcontext)`.
    let destroy: CuCtxDestroy = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, CuCtxDestroy>(destroy)
    };
    let mut dev = 0i32;
    // SAFETY: `dev` is a valid out-pointer; ordinal 0 exists (count > 0).
    if unsafe { get(&mut dev, 0) } != 0 {
        return out;
    }
    let mut ctx: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `ctx` is a valid out-pointer and `dev` a device handle from cuDeviceGet.
    if unsafe { create(&mut ctx, 0, dev) } == 0 {
        out.context_held = true;
        std::thread::sleep(hold);
        // SAFETY: `ctx` is the context created above, destroyed once.
        let _ = unsafe { destroy(ctx) };
    }
    out
}
