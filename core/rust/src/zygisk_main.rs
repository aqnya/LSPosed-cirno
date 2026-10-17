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

//! Rust replacement for `magisk-loader/src/main/jni/api/zygisk_main.cpp`.
//! The exported `zygisk_module_entry` symbol stays in C++ (see
//! `zygisk_bridge.cpp`) so this static library object is always pulled in.

use jni::objects::JValue;
use jni::sys::{jboolean, jint, jintArray, jobjectArray, jstring, JNI_FALSE};
use jni::JNIEnv;
use std::ffi::{c_char, c_int, c_long, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

type PreApp = unsafe extern "C" fn(*mut c_void, *mut AppSpecializeArgs);
type PostApp = unsafe extern "C" fn(*mut c_void, *const AppSpecializeArgs);
type PreServer = unsafe extern "C" fn(*mut c_void, *mut c_void);
type PostServer = unsafe extern "C" fn(*mut c_void, *const c_void);

#[repr(C)]
struct AppSpecializeArgs {
    uid: *mut jint,
    gid: *mut jint,
    gids: *mut jintArray,
    runtime_flags: *mut jint,
    rlimits: *mut jobjectArray,
    mount_external: *mut jint,
    se_info: *mut jstring,
    nice_name: *mut jstring,
    instruction_set: *mut jstring,
    app_data_dir: *mut jstring,
    fds_to_ignore: *mut jintArray,
    is_child_zygote: *mut jboolean,
    is_top_app: *mut jboolean,
    pkg_data_info_list: *mut jobjectArray,
    whitelisted_data_info_list: *mut jobjectArray,
    mount_data_dirs: *mut jboolean,
    mount_storage_dirs: *mut jboolean,
}

#[repr(C)]
pub struct ModuleAbi {
    api_version: c_long,
    implementation: *mut c_void,
    pre_app: PreApp,
    post_app: PostApp,
    pre_server: PreServer,
    post_server: PostServer,
}

#[repr(C)]
pub struct ApiTable {
    implementation: *mut c_void,
    register_module: Option<unsafe extern "C" fn(*mut ApiTable, *mut ModuleAbi) -> bool>,
    hook_jni_native_methods: Option<unsafe extern "C" fn()>,
    plt_hook_register: Option<unsafe extern "C" fn()>,
    exempt_fd: Option<unsafe extern "C" fn()>,
    plt_hook_commit: Option<unsafe extern "C" fn()>,
    connect_companion: Option<unsafe extern "C" fn()>,
    set_option: Option<unsafe extern "C" fn(*mut c_void, c_int)>,
    get_module_dir: Option<unsafe extern "C" fn()>,
    get_flags: Option<unsafe extern "C" fn()>,
}

extern "C" {
    fn lspd_magisk_loader_init();
    fn lspd_config_init();
    fn lspd_pre_app(
        env: *mut jni::sys::JNIEnv,
        uid: jint,
        gids: *mut jintArray,
        nice_name: *mut jstring,
        is_child_zygote: jboolean,
        app_data_dir: jstring,
    );
    fn lspd_post_app(
        env: *mut jni::sys::JNIEnv,
        nice_name: jstring,
        app_data_dir: jstring,
    );
    fn lspd_pre_server(env: *mut jni::sys::JNIEnv);
    fn lspd_post_server(env: *mut jni::sys::JNIEnv);
    fn __system_property_find(name: *const c_char) -> *const c_void;
}

static API: AtomicPtr<ApiTable> = AtomicPtr::new(ptr::null_mut());
static ENV: AtomicPtr<jni::sys::JNIEnv> = AtomicPtr::new(ptr::null_mut());
static UNLOAD: AtomicBool = AtomicBool::new(false);

static mut ABI: ModuleAbi = ModuleAbi {
    api_version: 4,
    implementation: ptr::null_mut(),
    pre_app,
    post_app,
    pre_server,
    post_server,
};

#[no_mangle]
pub unsafe extern "C" fn lspd_zygisk_entry(
    table: *mut ApiTable,
    env: *mut jni::sys::JNIEnv,
) {
    if table.is_null() {
        return;
    }
    let Some(register_module) = (*table).register_module else {
        return;
    };
    let abi = &mut *ptr::addr_of_mut!(ABI);
    abi.implementation = abi as *mut ModuleAbi as *mut c_void;
    if !register_module(table, abi) {
        return;
    }
    API.store(table, Ordering::Release);
    ENV.store(env, Ordering::Release);
    lspd_magisk_loader_init();
    lspd_config_init();
}

#[no_mangle]
pub extern "C" fn lspd_set_allow_unload(unload: c_int) {
    UNLOAD.store(unload != 0, Ordering::Release);
}

unsafe extern "C" fn pre_app(_: *mut c_void, args: *mut AppSpecializeArgs) {
    let Some(args) = args.as_ref() else {
        return;
    };
    let env = ENV.load(Ordering::Acquire);
    if env.is_null()
        || args.uid.is_null()
        || args.gids.is_null()
        || args.nice_name.is_null()
        || args.app_data_dir.is_null()
    {
        return;
    }
    let is_child_zygote = if args.is_child_zygote.is_null() {
        JNI_FALSE
    } else {
        *args.is_child_zygote
    };
    lspd_pre_app(
        env,
        *args.uid,
        args.gids,
        args.nice_name,
        is_child_zygote,
        *args.app_data_dir,
    );
}

unsafe extern "C" fn post_app(_: *mut c_void, args: *const AppSpecializeArgs) {
    let Some(args) = args.as_ref() else {
        return;
    };
    let env = ENV.load(Ordering::Acquire);
    if env.is_null() || args.nice_name.is_null() || args.app_data_dir.is_null() {
        return;
    }
    lspd_post_app(env, *args.nice_name, *args.app_data_dir);
    if UNLOAD.load(Ordering::Acquire) {
        set_option(1);
    }
}

unsafe extern "C" fn pre_server(_: *mut c_void, _: *mut c_void) {
    let env = ENV.load(Ordering::Acquire);
    if !env.is_null() {
        lspd_pre_server(env);
    }
}

unsafe extern "C" fn post_server(_: *mut c_void, _: *const c_void) {
    let env = ENV.load(Ordering::Acquire);
    if env.is_null() {
        return;
    }

    let property = b"ro.vendor.product.ztename\0";
    if !__system_property_find(property.as_ptr().cast()).is_null() {
        fix_argument_zero(env);
    }

    lspd_post_server(env);
    if UNLOAD.load(Ordering::Acquire) {
        set_option(1);
    }
}

unsafe fn fix_argument_zero(raw_env: *mut jni::sys::JNIEnv) {
    let Ok(mut env) = JNIEnv::from_raw(raw_env) else {
        return;
    };
    let Ok(name) = env.new_string("system_server") else {
        return;
    };
    let _ = env.call_static_method(
        "android/os/Process",
        "setArgV0",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&name)],
    );
}

unsafe fn set_option(option: c_int) {
    let table = API.load(Ordering::Acquire);
    if table.is_null() {
        return;
    }
    if let Some(set_option) = (*table).set_option {
        set_option((*table).implementation, option);
    }
}
