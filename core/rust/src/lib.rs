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

//! Rust part of `liblspd.so`.
//!
//! The native bridge JNI entry point lives here; the rest of the core is
//! still C++ and is called through a small C ABI.

use jni::objects::{JClass, JString};
use jni::JNIEnv;
use std::ffi::c_char;

mod elf_util;

extern "C" {
    fn lspd_register_native_lib(name: *const c_char, size: usize);
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_NativeAPI_recordNativeEntrypoint(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
) {
    if name.as_raw().is_null() {
        return;
    }
    if let Ok(name) = env.get_string(&name) {
        let name: String = name.into();
        let bytes = name.as_bytes();
        unsafe {
            lspd_register_native_lib(bytes.as_ptr().cast(), bytes.len());
        }
    }
}
