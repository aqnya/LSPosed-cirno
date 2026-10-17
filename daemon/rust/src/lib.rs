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

//! Rust takeover of the daemon-side `dex2oat.cpp` JNI entry points:
//! mounting/unmounting the dex2oat wrapper, the sockcreate SELinux context and the
//! abstract socket path.

use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use std::ffi::{c_char, c_int, c_void, CString};
use std::io;
use std::ptr;

const LOG_TAG: &[u8] = b"LSPosedDex2Oat\0";
const ANDROID_LOG_INFO: c_int = 4;
const ANDROID_LOG_ERROR: c_int = 6;
const SOCKET_NAME: &str = "5291374ceda0aef7c5d86cd2a4f6a3ac";

#[link(name = "log")]
extern "C" {
    fn __android_log_print(
        prio: c_int,
        tag: *const c_char,
        fmt: *const c_char,
        ...
    ) -> c_int;
}

fn log(priority: c_int, message: &str) {
    if let Ok(message) = CString::new(message) {
        unsafe {
            __android_log_print(
                priority,
                LOG_TAG.as_ptr().cast(),
                b"%s\0".as_ptr().cast(),
                message.as_ptr(),
            );
        }
    }
}

fn jstring_to_string(env: &mut JNIEnv, value: &JString) -> Option<String> {
    if value.as_raw().is_null() {
        return None;
    }
    env.get_string(value).ok().map(Into::into)
}

/// Equivalent to `realpath()`; falls back to the original path if it cannot be resolved.
fn absolute(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

unsafe fn bind_mount(source: &str, target: Option<&str>) {
    let Some(target) = target else {
        return;
    };
    let (Ok(source), Ok(target)) = (CString::new(source), CString::new(target)) else {
        return;
    };
    libc::mount(
        source.as_ptr(),
        target.as_ptr(),
        ptr::null(),
        libc::MS_BIND,
        ptr::null(),
    );
    libc::mount(
        ptr::null(),
        target.as_ptr(),
        ptr::null(),
        libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY,
        ptr::null(),
    );
}

unsafe fn unmount(target: Option<&str>) {
    let Some(target) = target else {
        return;
    };
    if let Ok(target) = CString::new(target) {
        libc::umount(target.as_ptr());
    }
}

unsafe fn exec_resetprop(args: &[&str]) -> ! {
    let args: Vec<CString> = args
        .iter()
        .map(|arg| CString::new(*arg).expect("resetprop argument"))
        .collect();
    let mut pointers: Vec<*const c_char> = args.iter().map(|arg| arg.as_ptr()).collect();
    pointers.push(ptr::null());
    libc::execvp(args[0].as_ptr(), pointers.as_ptr());
    log(
        ANDROID_LOG_ERROR,
        &format!("execvp resetprop failed: {}", io::Error::last_os_error()),
    );
    libc::exit(1)
}

fn write_all(fd: c_int, mut data: &[u8]) -> isize {
    while !data.is_empty() {
        let written = unsafe { libc::write(fd, data.as_ptr() as *const c_void, data.len()) };
        if written < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return -1;
        }
        data = &data[written as usize..];
    }
    0
}

fn setsockcreatecon_raw(context: Option<&str>) -> c_int {
    let path = format!(
        "/proc/self/task/{}/attr/sockcreate",
        unsafe { libc::gettid() }
    );
    let Ok(path) = CString::new(path) else {
        return -1;
    };
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return -1;
    }

    let result = match context {
        Some(context) => {
            let mut data = Vec::with_capacity(context.len() + 1);
            data.extend_from_slice(context.as_bytes());
            data.push(0);
            write_all(fd, &data)
        }
        None => 0,
    };

    unsafe { libc::close(fd) };
    if result < 0 {
        -1
    } else {
        0
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_service_Dex2OatService_doMountNative(
    mut env: JNIEnv,
    _this: JObject,
    enabled: jboolean,
    r32: JString,
    d32: JString,
    r64: JString,
    d64: JString,
) {
    let r32 = jstring_to_string(&mut env, &r32);
    let d32 = jstring_to_string(&mut env, &d32);
    let r64 = jstring_to_string(&mut env, &r64);
    let d64 = jstring_to_string(&mut env, &d64);
    let dex2oat32 = absolute("bin/dex2oat32");
    let dex2oat64 = absolute("bin/dex2oat64");

    unsafe {
        let pid = libc::fork();
        if pid > 0 {
            libc::waitpid(pid, ptr::null_mut(), 0);
            return;
        }
        if pid < 0 {
            log(
                ANDROID_LOG_ERROR,
                &format!("fork failed: {}", io::Error::last_os_error()),
            );
            return;
        }

        let mount_namespace = CString::new("/proc/1/ns/mnt").unwrap();
        let ns = libc::open(mount_namespace.as_ptr(), libc::O_RDONLY);
        if ns >= 0 {
            libc::setns(ns, libc::CLONE_NEWNS);
            libc::close(ns);
        }

        if enabled != 0 {
            log(ANDROID_LOG_INFO, "Enable dex2oat wrapper");
            bind_mount(&dex2oat32, r32.as_deref());
            bind_mount(&dex2oat32, d32.as_deref());
            bind_mount(&dex2oat64, r64.as_deref());
            bind_mount(&dex2oat64, d64.as_deref());
            exec_resetprop(&["resetprop", "--delete", "dalvik.vm.dex2oat-flags"]);
        } else {
            log(ANDROID_LOG_INFO, "Disable dex2oat wrapper");
            unmount(r32.as_deref());
            unmount(d32.as_deref());
            unmount(r64.as_deref());
            unmount(d64.as_deref());
            exec_resetprop(&[
                "resetprop",
                "dalvik.vm.dex2oat-flags",
                "--inline-max-code-units=0",
            ]);
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_service_Dex2OatService_setSockCreateContext(
    mut env: JNIEnv,
    _class: JClass,
    context: JString,
) -> jboolean {
    let context = jstring_to_string(&mut env, &context);
    if setsockcreatecon_raw(context.as_deref()) == 0 {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_service_Dex2OatService_getSockPath(
    mut env: JNIEnv,
    _this: JObject,
) -> jstring {
    env.new_string(SOCKET_NAME)
        .map(|string| string.into_raw())
        .unwrap_or(ptr::null_mut())
}
