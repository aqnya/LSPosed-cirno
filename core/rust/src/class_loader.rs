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

//! Rust implementation of `Context::FindClassFromLoader`.

use jni::objects::{JObject, JValue};
use jni::sys::{jclass, jobject, JNIEnv as RawJNIEnv};
use jni::JNIEnv;
use std::ffi::c_char;
use std::mem::ManuallyDrop;
use std::ptr;
use std::slice;

#[no_mangle]
pub unsafe extern "C" fn lspd_find_class_from_loader(
    raw_env: *mut RawJNIEnv,
    class_loader: jobject,
    class_name: *const c_char,
    class_name_len: usize,
) -> jclass {
    if raw_env.is_null() || class_loader.is_null() || class_name.is_null() {
        return ptr::null_mut();
    }
    let Ok(mut env) = JNIEnv::from_raw(raw_env) else {
        return ptr::null_mut();
    };
    let Ok(class_name) =
        std::str::from_utf8(slice::from_raw_parts(class_name as *const u8, class_name_len))
    else {
        return ptr::null_mut();
    };

    let loader = ManuallyDrop::new(JObject::from_raw(class_loader));
    match find_class(&mut env, &loader, class_name) {
        Some(class) => class.into_raw(),
        None => ptr::null_mut(),
    }
}

fn find_class<'local>(
    env: &mut JNIEnv<'local>,
    loader: &JObject,
    class_name: &str,
) -> Option<JObject<'local>> {
    let class_name = env.new_string(class_name).ok()?;
    let signature = "(Ljava/lang/String;)Ljava/lang/Class;";

    let value = match env.call_method(
        loader,
        "loadClass",
        signature,
        &[JValue::Object(&class_name)],
    ) {
        Ok(value) => value,
        Err(_) => {
            let _ = env.exception_clear();
            env.call_method(
                loader,
                "findClass",
                signature,
                &[JValue::Object(&class_name)],
            )
            .ok()?
        }
    };
    let class = value.l().ok()?;
    if class.is_null() {
        None
    } else {
        Some(class)
    }
}
