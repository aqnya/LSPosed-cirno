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

//! mmap-backed dex image used by `Context::PreloadedDex`.

use std::ffi::{c_int, c_void};
use std::ptr;

struct PreloadedDex {
    address: *mut c_void,
    size: usize,
}

#[no_mangle]
pub unsafe extern "C" fn lspd_preloaded_dex_create(
    fd: c_int,
    size: usize,
) -> *mut c_void {
    if fd < 0 || size == 0 {
        return ptr::null_mut();
    }
    let address = libc::mmap(
        ptr::null_mut(),
        size,
        libc::PROT_READ,
        libc::MAP_SHARED,
        fd,
        0,
    );
    if address == libc::MAP_FAILED {
        return ptr::null_mut();
    }
    Box::into_raw(Box::new(PreloadedDex { address, size })) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn lspd_preloaded_dex_destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let dex = Box::from_raw(handle as *mut PreloadedDex);
    libc::munmap(dex.address, dex.size);
}

#[no_mangle]
pub unsafe extern "C" fn lspd_preloaded_dex_data(handle: *mut c_void) -> *mut c_void {
    if handle.is_null() {
        return ptr::null_mut();
    }
    (*(handle as *const PreloadedDex)).address
}

#[no_mangle]
pub unsafe extern "C" fn lspd_preloaded_dex_size(handle: *mut c_void) -> usize {
    if handle.is_null() {
        return 0;
    }
    (*(handle as *const PreloadedDex)).size
}
