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
 * Copyright (C) 2020 EdXposed Contributors
 * Copyright (C) 2023 - 2026 LSPosed Contributors
 */

//! Rust replacement for `core/src/main/jni/src/native_api.cpp`: the
//! `do_dlopen` hook and the module native-library registry.
//!
//! Dobby itself stays behind two tiny C++ wrappers so the Rust unit tests do
//! not need to link the Dobby static library on the host.

use jni::objects::{JClass, JString};
use jni::sys::{jobject, JNIEnv as RawJNIEnv};
use jni::{JNIEnv, NativeMethod};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};

type HookFunc = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> c_int;
type UnhookFunc = unsafe extern "C" fn(*mut c_void) -> c_int;
type NativeOnModuleLoaded = unsafe extern "C" fn(*const c_char, *mut c_void);
type NativeInit =
    unsafe extern "C" fn(*const NativeAPIEntries) -> Option<NativeOnModuleLoaded>;

#[repr(C)]
struct NativeAPIEntries {
    version: u32,
    hook_func: HookFunc,
    unhook_func: UnhookFunc,
}

extern "C" {
    fn lspd_get_current_class_loader() -> jobject;
    fn lspd_native_bridge_prefix(length: *mut usize) -> *const c_char;
    fn lspd_native_hook(
        function: *mut c_void,
        replacement: *mut c_void,
        backup: *mut *mut c_void,
    ) -> c_int;
    fn lspd_native_unhook(function: *mut c_void) -> c_int;
}

static ENTRIES: OnceLock<usize> = OnceLock::new();
static INSTALLED: OnceLock<bool> = OnceLock::new();
static BACKUP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static MODULE_LIBS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static CALLBACKS: OnceLock<Mutex<Vec<NativeOnModuleLoaded>>> = OnceLock::new();

fn module_libs() -> &'static Mutex<Vec<String>> {
    MODULE_LIBS.get_or_init(|| Mutex::new(Vec::new()))
}

fn callbacks() -> &'static Mutex<Vec<NativeOnModuleLoaded>> {
    CALLBACKS.get_or_init(|| Mutex::new(Vec::new()))
}

fn entries() -> *const NativeAPIEntries {
    *ENTRIES.get_or_init(|| unsafe {
        let page = libc::mmap(
            ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_ANONYMOUS | libc::MAP_SHARED,
            -1,
            0,
        );
        if page == libc::MAP_FAILED {
            return 0;
        }
        ptr::write(
            page as *mut NativeAPIEntries,
            NativeAPIEntries {
                version: 2,
                hook_func: lspd_native_hook,
                unhook_func: lspd_native_unhook,
            },
        );
        libc::mprotect(page, 4096, libc::PROT_READ);
        page as usize
    }) as *const NativeAPIEntries
}

fn ensure_installed() -> bool {
    *INSTALLED.get_or_init(|| unsafe {
        let module = b"/linker";
        let handle = crate::elf_util::lspd_elf_create(module.as_ptr().cast(), module.len());
        if handle.is_null() {
            return false;
        }
        let symbol = b"__dl__Z9do_dlopenPKciPK17android_dlextinfoPKv";
        let address = crate::elf_util::lspd_elf_lookup(
            handle,
            symbol.as_ptr().cast(),
            symbol.len(),
        );
        crate::elf_util::lspd_elf_destroy(handle);
        if address == 0 {
            return false;
        }

        // Create the entries page before installing the hook: a concurrent
        // dlopen could invoke the hook immediately after DobbyHook returns.
        entries();

        let mut backup = ptr::null_mut();
        let result = lspd_native_hook(
            address as *mut c_void,
            do_dlopen_hook as *mut c_void,
            &mut backup,
        );
        if result != 0 {
            return false;
        }
        BACKUP.store(backup, Ordering::Release);
        true
    })
}

pub fn register_native_lib(name: &str) {
    if !ensure_installed() {
        return;
    }
    if let Ok(mut libs) = module_libs().lock() {
        libs.push(name.to_string());
    }
}

unsafe extern "C" fn do_dlopen_hook(
    name: *const c_char,
    flags: c_int,
    extinfo: *const c_void,
    caller_address: *const c_void,
) -> *mut c_void {
    let backup = BACKUP.load(Ordering::Acquire);
    if backup.is_null() {
        return ptr::null_mut();
    }
    let backup: unsafe extern "C" fn(
        *const c_char,
        c_int,
        *const c_void,
        *const c_void,
    ) -> *mut c_void = std::mem::transmute(backup);
    let handle = backup(name, flags, extinfo, caller_address);
    if handle.is_null() {
        return handle;
    }

    let name_string = if name.is_null() {
        "NULL".to_string()
    } else {
        CStr::from_ptr(name).to_string_lossy().into_owned()
    };

    let libraries = module_libs()
        .lock()
        .map(|libs| libs.clone())
        .unwrap_or_default();
    for library in libraries {
        if !name_string.ends_with(&library) {
            continue;
        }
        let Ok(symbol) = CString::new("native_init") else {
            break;
        };
        let native_init = libc::dlsym(handle, symbol.as_ptr());
        if native_init.is_null() {
            break;
        }
        let native_init: NativeInit = std::mem::transmute(native_init);
        if let Some(callback) = native_init(entries()) {
            if let Ok(mut callbacks) = callbacks().lock() {
                callbacks.push(callback);
            }
            return handle;
        }
    }

    let callbacks = callbacks()
        .lock()
        .map(|callbacks| callbacks.clone())
        .unwrap_or_default();
    for callback in callbacks {
        callback(name, handle);
    }
    handle
}

#[no_mangle]
pub unsafe extern "C" fn lspd_register_native_api(raw_env: *mut RawJNIEnv) {
    if raw_env.is_null() {
        return;
    }
    let Ok(mut env) = JNIEnv::from_raw(raw_env) else {
        return;
    };
    let class_loader = lspd_get_current_class_loader();
    if class_loader.is_null() {
        return;
    }

    let mut prefix_length = 0usize;
    let prefix = lspd_native_bridge_prefix(&mut prefix_length);
    if prefix.is_null() {
        return;
    }
    let prefix = slice::from_raw_parts(prefix as *const u8, prefix_length);
    let mut class_name = Vec::with_capacity(prefix.len() + 9);
    class_name.extend_from_slice(prefix);
    class_name.extend_from_slice(b"NativeAPI");

    let class = crate::class_loader::lspd_find_class_from_loader(
        env.get_native_interface(),
        class_loader,
        class_name.as_ptr().cast(),
        class_name.len(),
    );
    if class.is_null() {
        return;
    }
    let class = JClass::from_raw(class);
    let methods = [NativeMethod {
        name: "recordNativeEntrypoint".into(),
        sig: "(Ljava/lang/String;)V".into(),
        fn_ptr: Java_org_lsposed_lspd_nativebridge_NativeAPI_recordNativeEntrypoint
            as *const () as *mut c_void,
    }];
    let _ = env.register_native_methods(&class, &methods);
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
        register_native_lib(&name);
    }
}
