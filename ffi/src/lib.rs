//! C API of [`nxformat_packer::cli`]; declared in `include/nxformat.h`.
#![allow(clippy::missing_safety_doc)]

use nxformat_packer::{cli, CancellationToken};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

pub const NXFORMAT_ABI_VERSION: u32 = 1;
pub const NXFORMAT_OK: c_int = 0;
pub const NXFORMAT_FAILED: c_int = 1;
pub const NXFORMAT_CANCELLED: c_int = 2;
pub const NXFORMAT_INVALID: c_int = 3;
pub const NXFORMAT_INTERNAL: c_int = 4;

pub type LineFn =
    Option<unsafe extern "C" fn(user: *mut c_void, is_error: c_int, line: *const c_char)>;

/// Opaque cancel handle.
#[allow(non_camel_case_types)]
pub struct nxformat_cancel(CancellationToken);

#[no_mangle]
pub extern "C" fn nxformat_abi_version() -> u32 {
    NXFORMAT_ABI_VERSION
}

#[no_mangle]
pub extern "C" fn nxformat_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

#[no_mangle]
pub extern "C" fn nxformat_cancel_new() -> *mut nxformat_cancel {
    Box::into_raw(Box::new(nxformat_cancel(CancellationToken::default())))
}

#[no_mangle]
pub unsafe extern "C" fn nxformat_cancel_set(cancel: *mut nxformat_cancel) {
    if let Some(cancel) = cancel.as_ref() {
        cancel.0.cancel();
    }
}

#[no_mangle]
pub unsafe extern "C" fn nxformat_cancel_free(cancel: *mut nxformat_cancel) {
    if !cancel.is_null() {
        drop(Box::from_raw(cancel));
    }
}

/// The callback and its user pointer, called from the run's threads.
struct Callback {
    on_line: unsafe extern "C" fn(*mut c_void, c_int, *const c_char),
    user: *mut c_void,
    mutex: Mutex<()>,
}
// The caller owns `user` and promises it may be used until the run returns.
unsafe impl Send for Callback {}
unsafe impl Sync for Callback {}

impl Callback {
    fn call(&self, is_error: bool, line: &str) {
        // An interior NUL would cut the line; such lines do not occur.
        let line = CString::new(line.replace('\0', " ")).unwrap_or_default();
        // Progress and command output may arrive on different worker threads.
        // A frontend's per-run callback state must never be entered twice.
        let _guard = self.mutex.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { (self.on_line)(self.user, c_int::from(is_error), line.as_ptr()) };
    }
}

fn write_error(error: *mut c_char, size: usize, message: &str) {
    if error.is_null() || size == 0 {
        return;
    }
    let mut len = message.len().min(size - 1);
    while !message.is_char_boundary(len) {
        len -= 1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(message.as_ptr(), error.cast::<u8>(), len);
        *error.add(len) = 0;
    }
}

#[no_mangle]
pub unsafe extern "C" fn nxformat_run(
    args: *const *const c_char,
    count: usize,
    on_line: LineFn,
    user: *mut c_void,
    cancel: *mut nxformat_cancel,
    error: *mut c_char,
    error_size: usize,
) -> c_int {
    if count != 0 && args.is_null() {
        write_error(error, error_size, "null argument list");
        return NXFORMAT_INVALID;
    }
    let mut argv = Vec::new();
    if argv.try_reserve_exact(count).is_err() {
        write_error(error, error_size, "argument count exceeds available memory");
        return NXFORMAT_INVALID;
    }
    for i in 0..count {
        let arg = *args.add(i);
        if arg.is_null() {
            write_error(error, error_size, &format!("argument {i} is null"));
            return NXFORMAT_INVALID;
        }
        match CStr::from_ptr(arg).to_str() {
            Ok(arg) => argv.push(arg.to_owned()),
            Err(_) => {
                write_error(error, error_size, &format!("argument {i} is not UTF-8"));
                return NXFORMAT_INVALID;
            }
        }
    }
    let sink: Option<cli::Sink> = on_line.map(|on_line| {
        let callback = Callback {
            on_line,
            user,
            mutex: Mutex::new(()),
        };
        Arc::new(move |is_error: bool, line: &str| callback.call(is_error, line)) as cli::Sink
    });
    let token = cancel.as_ref().map(|c| c.0.clone());
    let watch = token.clone();
    match catch_unwind(AssertUnwindSafe(|| cli::run(argv, sink, token))) {
        Ok(Ok(())) => NXFORMAT_OK,
        Ok(Err(e)) => {
            write_error(error, error_size, &e.to_string());
            if watch.is_some_and(|t| t.is_cancelled()) {
                NXFORMAT_CANCELLED
            } else {
                NXFORMAT_FAILED
            }
        }
        Err(panic) => {
            let what = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            write_error(error, error_size, &format!("internal error: {what}"));
            NXFORMAT_INTERNAL
        }
    }
}

pub type ReadFn = Option<
    unsafe extern "C" fn(user: *mut c_void, offset: u64, buffer: *mut c_void, size: usize) -> c_int,
>;

/// A plaintext source read through the caller's callback.
#[allow(non_camel_case_types)]
pub struct nxformat_source {
    // Kept for its Drop, which unregisters the source.
    #[allow(dead_code)]
    registration: nxformat_packer::source_file::SourceRegistration,
    path: CString,
}

struct CallerReader {
    read: unsafe extern "C" fn(*mut c_void, u64, *mut c_void, usize) -> c_int,
    user: *mut c_void,
}
// The caller promises `read` and `user` may be used from any thread until the
// source is freed; calls for one source are serialized by nxformat.
unsafe impl Send for CallerReader {}

impl nxformat_packer::source_file::SourceReader for CallerReader {
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
        let status =
            unsafe { (self.read)(self.user, offset, buffer.as_mut_ptr().cast(), buffer.len()) };
        if status == 0 {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "the caller could not read {} bytes at {offset}",
                buffer.len()
            )))
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn nxformat_source_new(
    name: *const c_char,
    size: u64,
    read: ReadFn,
    user: *mut c_void,
) -> *mut nxformat_source {
    let (Some(read), false) = (read, name.is_null()) else {
        return std::ptr::null_mut();
    };
    let Ok(name) = CStr::from_ptr(name).to_str() else {
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let registration = nxformat_packer::source_file::SourceRegistration::new(
            name,
            size,
            Box::new(CallerReader { read, user }),
        );
        let path = CString::new(registration.path().to_string_lossy().into_owned())
            .expect("source paths have no NUL");
        Box::into_raw(Box::new(nxformat_source { registration, path }))
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn nxformat_source_path(source: *const nxformat_source) -> *const c_char {
    source
        .as_ref()
        .map_or(std::ptr::null(), |s| s.path.as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn nxformat_source_free(source: *mut nxformat_source) {
    if !source.is_null() {
        drop(Box::from_raw(source));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn one_runs_callbacks_are_serialized_across_worker_threads() {
        struct State {
            active: AtomicUsize,
            peak: AtomicUsize,
            calls: AtomicUsize,
        }
        unsafe extern "C" fn on_line(user: *mut c_void, _: c_int, _: *const c_char) {
            let state = &*user.cast::<State>();
            let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
            state.peak.fetch_max(active, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(1));
            state.calls.fetch_add(1, Ordering::SeqCst);
            state.active.fetch_sub(1, Ordering::SeqCst);
        }
        let state = State {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
        };
        let callback = Arc::new(Callback {
            on_line,
            user: (&state as *const State).cast_mut().cast(),
            mutex: Mutex::new(()),
        });
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let callback = callback.clone();
                std::thread::spawn(move || {
                    for _ in 0..8 {
                        callback.call(false, "output");
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(state.peak.load(Ordering::SeqCst), 1);
        assert_eq!(state.calls.load(Ordering::SeqCst), 64);
    }
}
