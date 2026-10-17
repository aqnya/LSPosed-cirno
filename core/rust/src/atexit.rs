/*
 * This file is part of LSPosed.
 *
 * LSPosed is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * LSPosed is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with LSPosed.  If not, see <https://www.gnu.org/licenses/>.
 *
 * Copyright (C) 2023 - 2026 LSPosed Contributors
 */

//! Rust implementation of the module-local `__cxa_atexit`/`__cxa_finalize`
//! replacement. The C++ `atexit.cpp` keeps only the exported symbols and
//! forwards to these functions.

use std::ffi::c_void;
use std::sync::Mutex;

type AtexitFn = unsafe extern "C" fn(*mut c_void);

static ENTRIES: Mutex<Vec<(AtexitFn, usize)>> = Mutex::new(Vec::new());

#[no_mangle]
pub extern "C" fn lspd_rust_cxa_atexit(
    function: Option<AtexitFn>,
    argument: *mut c_void,
) -> std::ffi::c_int {
    let Some(function) = function else {
        return -1;
    };
    match ENTRIES.lock() {
        Ok(mut entries) => {
            entries.push((function, argument as usize));
            0
        }
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn lspd_rust_cxa_finalize() {
    loop {
        let entry = match ENTRIES.lock() {
            Ok(mut entries) => entries.pop(),
            Err(_) => return,
        };
        let Some((function, argument)) = entry else {
            break;
        };
        unsafe { function(argument as *mut c_void) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn callback(_: *mut c_void) {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn registers_and_finalizes() {
        CALLS.store(0, Ordering::SeqCst);
        assert_eq!(lspd_rust_cxa_atexit(Some(callback), std::ptr::null_mut()), 0);
        lspd_rust_cxa_finalize();
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(lspd_rust_cxa_atexit(None, std::ptr::null_mut()), -1);
    }
}
