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

//! Rust implementation of `Context::InitHooks`' dex-trust loop and
//! registration sequence.

use jni::objects::{JObject, JObjectArray};
use jni::sys::{jboolean, jobject, JNIEnv as RawJNIEnv};
use jni::JNIEnv;
use std::mem::ManuallyDrop;

extern "C" {
    fn lspd_make_dex_file_trusted(env: *mut RawJNIEnv, cookie: jobject) -> jboolean;
    fn lspd_register_resources_hook(env: *mut RawJNIEnv);
    fn lspd_register_hook_bridge(env: *mut RawJNIEnv);
    fn lspd_register_native_api(env: *mut RawJNIEnv);
}

#[no_mangle]
pub unsafe extern "C" fn lspd_init_hooks(
    raw_env: *mut RawJNIEnv,
    class_loader: jobject,
) {
    if raw_env.is_null() {
        return;
    }
    let Ok(mut env) = JNIEnv::from_raw(raw_env) else {
        return;
    };
    let Ok(true) = trust_dex(&mut env, raw_env, class_loader) else {
        return;
    };
    lspd_register_resources_hook(raw_env);
    lspd_register_hook_bridge(raw_env);
    lspd_register_native_api(raw_env);
}

unsafe fn trust_dex(
    env: &mut JNIEnv,
    raw_env: *mut RawJNIEnv,
    class_loader: jobject,
) -> jni::errors::Result<bool> {
    if class_loader.is_null() {
        return Ok(false);
    }
    let loader = ManuallyDrop::new(JObject::from_raw(class_loader));

    let path_list = env
        .get_field(&*loader, "pathList", "Ldalvik/system/DexPathList;")?
        .l()?;
    if path_list.is_null() {
        return Ok(false);
    }

    let elements = env
        .get_field(
            &path_list,
            "dexElements",
            "[Ldalvik/system/DexPathList$Element;",
        )?
        .l()?;
    if elements.is_null() {
        return Ok(false);
    }
    let elements = JObjectArray::from(elements);
    let length = env.get_array_length(&elements)?;

    for index in 0..length {
        let element = env.get_object_array_element(&elements, index)?;
        if element.is_null() {
            continue;
        }
        let dex_file = env
            .get_field(&element, "dexFile", "Ldalvik/system/DexFile;")?
            .l()?;
        if dex_file.is_null() {
            continue;
        }
        let cookie = env.get_field(&dex_file, "mCookie", "Ljava/lang/Object;")?.l()?;
        if cookie.is_null() {
            continue;
        }
        lspd_make_dex_file_trusted(raw_env, cookie.as_raw());
    }

    Ok(true)
}
