//! NVML access for NVIDIA graphics cards.
//!
//! The driver library is loaded with `dlopen` at runtime and is never a link
//! time dependency. Machines without a card, with a different card, or with a
//! driver mid-upgrade simply report no GPU; nothing crashes and nothing has to
//! be rebuilt.
//!
//! Handles are kept as `usize` rather than raw pointers so the sampler stays
//! `Send` and can live inside a tokio task without an `unsafe impl`.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr};

use crate::config::MILLIWATTS_PER_WATT;
use crate::power::SourceError;

type NvmlReturn = c_int;
const NVML_SUCCESS: NvmlReturn = 0;
const NAME_BUFFER: usize = 96;

type InitFn = unsafe extern "C" fn() -> NvmlReturn;
type ShutdownFn = unsafe extern "C" fn() -> NvmlReturn;
type CountFn = unsafe extern "C" fn(*mut c_uint) -> NvmlReturn;
type HandleFn = unsafe extern "C" fn(c_uint, *mut *mut c_void) -> NvmlReturn;
type NameFn = unsafe extern "C" fn(*mut c_void, *mut c_char, c_uint) -> NvmlReturn;
type PowerFn = unsafe extern "C" fn(*mut c_void, *mut c_uint) -> NvmlReturn;

/// An initialised NVML session.
pub struct Nvml {
    // Held only so the symbols below stay mapped for the life of the session.
    _library: libloading::Library,
    shutdown: ShutdownFn,
    get_power_usage: PowerFn,
    /// Opaque `nvmlDevice_st *` handles, stored as integers.
    handles: Vec<usize>,
    names: Vec<String>,
}

impl Nvml {
    /// Open the driver and initialise a session, or return `None`.
    ///
    /// Failure is normal and expected on most machines, so it is silent apart
    /// from a debug log.
    pub fn open() -> Option<Self> {
        // `libnvidia-ml.so.1` is the runtime soname; the unversioned name is
        // only installed by the development package, so it is tried second.
        let library = match unsafe {
            libloading::Library::new("libnvidia-ml.so.1")
                .or_else(|_| libloading::Library::new("libnvidia-ml.so"))
        } {
            Ok(library) => library,
            Err(err) => {
                crate::logging::debug(&format!("NVML unavailable: {err}"));
                return None;
            }
        };

        unsafe {
            let init: InitFn = *library.get(b"nvmlInit_v2\0").ok()?;
            if init() != NVML_SUCCESS {
                crate::logging::debug("nvmlInit_v2 failed");
                return None;
            }
            let shutdown: ShutdownFn = *library.get(b"nvmlShutdown\0").ok()?;

            // NVML is initialised now. Every early return below has to shut it
            // down again, or the driver keeps a reference count.
            match Self::collect(library, shutdown) {
                Some(session) => Some(session),
                None => {
                    shutdown();
                    None
                }
            }
        }
    }

    unsafe fn collect(library: libloading::Library, shutdown: ShutdownFn) -> Option<Self> {
        let count_fn: CountFn = *library.get(b"nvmlDeviceGetCount_v2\0").ok()?;
        let handle_by_index: HandleFn = *library.get(b"nvmlDeviceGetHandleByIndex_v2\0").ok()?;
        let get_name: NameFn = *library.get(b"nvmlDeviceGetName\0").ok()?;
        let get_power_usage: PowerFn = *library.get(b"nvmlDeviceGetPowerUsage\0").ok()?;

        let mut count: c_uint = 0;
        if count_fn(&mut count) != NVML_SUCCESS || count == 0 {
            return None;
        }

        let mut handles = Vec::with_capacity(count as usize);
        let mut names = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut handle: *mut c_void = std::ptr::null_mut();
            if handle_by_index(index, &mut handle) != NVML_SUCCESS || handle.is_null() {
                continue;
            }
            let mut buffer = [0 as c_char; NAME_BUFFER];
            let name =
                if get_name(handle, buffer.as_mut_ptr(), NAME_BUFFER as c_uint) == NVML_SUCCESS {
                    CStr::from_ptr(buffer.as_ptr())
                        .to_string_lossy()
                        .into_owned()
                } else {
                    format!("GPU {index}")
                };
            handles.push(handle as usize);
            names.push(name);
        }

        if handles.is_empty() {
            return None;
        }

        Some(Self {
            _library: library,
            shutdown,
            get_power_usage,
            handles,
            names,
        })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Total draw of every card, in watts.
    pub fn watts(&self) -> Result<f64, SourceError> {
        let mut total = 0.0;
        for &handle in &self.handles {
            let mut milliwatts: c_uint = 0;
            let status = unsafe { (self.get_power_usage)(handle as *mut c_void, &mut milliwatts) };
            if status == NVML_SUCCESS {
                total += milliwatts as f64 / MILLIWATTS_PER_WATT;
            } else {
                // Some virtualised and laptop GPUs do not report power at all.
                crate::logging::debug(&format!("nvmlDeviceGetPowerUsage failed with {status}"));
            }
        }
        Ok(total)
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        unsafe { (self.shutdown)() };
    }
}
