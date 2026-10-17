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

//! Process-wide cached handles for libart and libbinder.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Mutex;

static ART: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static BINDER: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static LOCK: Mutex<()> = Mutex::new(());

unsafe fn get(cache: &AtomicPtr<c_void>, name: &[u8]) -> *mut c_void {
    let pointer = cache.load(Ordering::Acquire);
    if !pointer.is_null() {
        return pointer;
    }

    let _guard = LOCK.lock();
    let pointer = cache.load(Ordering::Acquire);
    if !pointer.is_null() {
        return pointer;
    }

    let pointer = crate::elf_util::lspd_elf_create(name.as_ptr().cast(), name.len());
    cache.store(pointer, Ordering::Release);
    pointer
}

#[no_mangle]
pub unsafe extern "C" fn lspd_symbol_cache_get_art() -> *mut c_void {
    get(&ART, b"libart.so")
}

#[no_mangle]
pub unsafe extern "C" fn lspd_symbol_cache_release_art() {
    let _guard = LOCK.lock();
    let pointer = ART.swap(ptr::null_mut(), Ordering::AcqRel);
    if !pointer.is_null() {
        crate::elf_util::lspd_elf_destroy(pointer);
    }
}

#[no_mangle]
pub unsafe extern "C" fn lspd_symbol_cache_get_binder() -> *mut c_void {
    get(&BINDER, b"libbinder.so")
}

#[no_mangle]
pub unsafe extern "C" fn lspd_symbol_cache_release_binder() {
    let _guard = LOCK.lock();
    let pointer = BINDER.swap(ptr::null_mut(), Ordering::AcqRel);
    if !pointer.is_null() {
        crate::elf_util::lspd_elf_destroy(pointer);
    }
}
