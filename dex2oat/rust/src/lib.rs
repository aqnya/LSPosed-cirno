// This file is part of LSPosed.
//
// LSPosed is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// LSPosed is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with LSPosed.  If not, see <https://www.gnu.org/licenses/>.
//
// Copyright (C) 2020 EdXposed Contributors
// Copyright (C) 2021 - 2026 LSPosed Contributors

//! Rust takeover of the `libpreload.so` dex2oat shim.
//!
//! `dex2oat` is started with `LD_PRELOAD=libpreload.so`. The wrapper replaces
//! `art::CompilerOptions::CompilerOptions()`, lets the original constructor run
//! and then patches the `inline_max_code_units_` field so that dex2oat does not
//! reject the (unset) value. Finally it clears `LD_PRELOAD` so child processes
//! do not inherit the shim.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::mem::{size_of, transmute};
use std::ptr;

const ANDROID_LOG_ERROR: c_int = 6;
const LOG_TAG: &[u8] = b"PRELOAD\0";
const CONSTRUCTOR: &[u8] = b"_ZN3art15CompilerOptionsC1Ev\0";
const RTLD_NEXT: *mut c_void = -1isize as *mut c_void;

const MAX_ITERATIONS: usize = 10;

type Constructor = unsafe extern "C" fn(*mut c_void);

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
    fn unsetenv(name: *const c_char) -> c_int;
}

#[link(name = "log")]
extern "C" {
    fn __android_log_print(
        prio: c_int,
        tag: *const c_char,
        fmt: *const c_char,
        ...
    ) -> c_int;
}

unsafe fn log_error(message: &str) {
    if let Ok(message) = CString::new(message) {
        __android_log_print(
            ANDROID_LOG_ERROR,
            LOG_TAG.as_ptr().cast(),
            b"%s\0".as_ptr().cast(),
            message.as_ptr(),
        );
    }
}

/// Replacement for `art::CompilerOptions::CompilerOptions()` (C1 complete-object
/// constructor, exported with its original mangled name).
#[no_mangle]
pub unsafe extern "C" fn _ZN3art15CompilerOptionsC1Ev(this: *mut c_void) {
    let original = dlsym(RTLD_NEXT, CONSTRUCTOR.as_ptr().cast());
    if original.is_null() {
        let error = dlerror();
        let detail = if error.is_null() {
            "unknown error".into()
        } else {
            CStr::from_ptr(error).to_string_lossy().into_owned()
        };
        log_error(&format!(
            "get sym _ZN3art15CompilerOptionsC1Ev, error: {detail}"
        ));
        return;
    }

    let constructor: Constructor = transmute(original);
    constructor(this);

    let max_offset = size_of::<usize>() * MAX_ITERATIONS;
    let mut offset = 0;
    while offset <= max_offset {
        if offset == max_offset {
            log_error("offsetof inline_max_code_units_ not found!");
            break;
        }

        let member = (this as *mut u8).add(offset) as *mut usize;
        if unsafe { ptr::read(member) } == usize::MAX {
            unsafe { ptr::write(member, 0) };
            break;
        }

        offset += size_of::<usize>();
    }

    if let Ok(name) = CString::new("LD_PRELOAD") {
        unsetenv(name.as_ptr());
    }
}
