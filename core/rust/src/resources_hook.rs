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

//! Rust rewrite of the ResourcesHook JNI layer. The Android private
//! ResXMLParser layout and dex_builder stay behind a small C ABI.

use jni::objects::{GlobalRef, JClass, JObject, JString};
use jni::sys::{
    jboolean, jclass, jint, jlong, jmethodID, jobject, jobjectArray, jstring, JNIEnv as RawJNIEnv,
    JNI_FALSE, JNI_TRUE,
};
use jni::{JNIEnv, NativeMethod};
use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::OnceLock;

struct XResources {
    class: GlobalRef,
    class_name: String,
    translate_attr: usize,
    translate_res: usize,
}

static STATE: OnceLock<XResources> = OnceLock::new();

extern "C" {
    fn lspd_get_current_class_loader() -> jobject;
    fn lspd_native_bridge_prefix(length: *mut usize) -> *const c_char;
    fn lspd_resources_class_name(length: *mut usize) -> *const c_char;
    fn lspd_resources_prepare_symbols() -> jboolean;
    fn lspd_resources_make_inheritable(env: *mut RawJNIEnv, clazz: jclass) -> jboolean;
    fn lspd_resources_build_dummy(
        env: *mut RawJNIEnv,
        parent: jobject,
        resource_super: jstring,
        typed_array_super: jstring,
    ) -> jobject;
    fn lspd_resources_rewrite_xml(
        env: *mut RawJNIEnv,
        parser: jlong,
        original: jobject,
        replacement: jobject,
        xresources_class: jobject,
        translate_attr: jmethodID,
        translate_res: jmethodID,
    );
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_ResourcesHook_initXResourcesNative(
    mut env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let mut name_length = 0usize;
    let name_ptr = unsafe { lspd_resources_class_name(&mut name_length) };
    if name_ptr.is_null() {
        return JNI_FALSE;
    }
    let class_name = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(
            name_ptr as *const u8,
            name_length,
        ))
    }
    .unwrap_or("")
    .to_string();

    let loader = unsafe { lspd_get_current_class_loader() };
    if loader.is_null() {
        return JNI_FALSE;
    }
    let class = unsafe {
        crate::class_loader::lspd_find_class_from_loader(
            env.get_native_interface(),
            loader,
            class_name.as_ptr().cast(),
            class_name.len(),
        )
    };
    if class.is_null() {
        return JNI_FALSE;
    }
    let class = unsafe { JClass::from_raw(class) };
    let class_global = match env.new_global_ref(&class) {
        Ok(class) => class,
        Err(_) => return JNI_FALSE,
    };

    let res_signature = format!("(IL{};Landroid/content/res/Resources;)I", class_name);
    let attr_signature = format!("(Ljava/lang/String;L{};)I", class_name);
    let translate_res = match env.get_static_method_id(&class, "translateResId", res_signature) {
        Ok(id) => id.into_raw() as usize,
        Err(_) => return JNI_FALSE,
    };
    let translate_attr = match env.get_static_method_id(&class, "translateAttrId", attr_signature) {
        Ok(id) => id.into_raw() as usize,
        Err(_) => return JNI_FALSE,
    };

    if unsafe { lspd_resources_prepare_symbols() } == JNI_FALSE {
        return JNI_FALSE;
    }

    if STATE.get().is_some() {
        return JNI_TRUE;
    }
    match STATE.set(XResources {
        class: class_global,
        class_name,
        translate_attr,
        translate_res,
    }) {
        Ok(()) => JNI_TRUE,
        Err(_) => JNI_FALSE,
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_ResourcesHook_makeInheritable(
    mut env: JNIEnv,
    _class: JClass,
    target_class: JClass,
) -> jboolean {
    if target_class.is_null() {
        return JNI_FALSE;
    }
    unsafe { lspd_resources_make_inheritable(env.get_native_interface(), target_class.as_raw()) }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_ResourcesHook_buildDummyClassLoader(
    mut env: JNIEnv,
    _class: JClass,
    parent: JObject,
    resource_super_class: JString,
    typed_array_super_class: JString,
) -> jobject {
    unsafe {
        lspd_resources_build_dummy(
            env.get_native_interface(),
            parent.as_raw(),
            resource_super_class.as_raw(),
            typed_array_super_class.as_raw(),
        )
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_ResourcesHook_rewriteXmlReferencesNative(
    mut env: JNIEnv,
    _class: JClass,
    parser: jlong,
    original: JObject,
    replacement: JObject,
) {
    let Some(state) = STATE.get() else {
        return;
    };
    let class = state.class.as_obj().as_raw();
    let translate_attr = state.translate_attr as jmethodID;
    let translate_res = state.translate_res as jmethodID;

    unsafe {
        lspd_resources_rewrite_xml(
            env.get_native_interface(),
            parser,
            original.as_raw(),
            replacement.as_raw(),
            class,
            translate_attr,
            translate_res,
        );
    }
}

#[no_mangle]
pub unsafe extern "C" fn lspd_register_resources_hook(raw_env: *mut RawJNIEnv) {
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
    let prefix = std::slice::from_raw_parts(prefix as *const u8, prefix_length);
    let mut class_name = Vec::with_capacity(prefix.len() + 13);
    class_name.extend_from_slice(prefix);
    class_name.extend_from_slice(b"ResourcesHook");
    let class = crate::class_loader::lspd_find_class_from_loader(
        raw_env,
        class_loader,
        class_name.as_ptr().cast(),
        class_name.len(),
    );
    if class.is_null() {
        return;
    }
    let class = unsafe { JClass::from_raw(class) };

    let mut xresources_length = 0usize;
    let xresources_ptr = lspd_resources_class_name(&mut xresources_length);
    let xresources = if xresources_ptr.is_null() {
        String::new()
    } else {
        std::str::from_utf8(std::slice::from_raw_parts(
            xresources_ptr as *const u8,
            xresources_length,
        ))
        .unwrap_or("")
        .to_string()
    };
    let rewrite_signature = format!("(JL{};Landroid/content/res/Resources;)V", xresources);

    let methods = [
        NativeMethod { name: "initXResourcesNative".into(), sig: "()Z".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_ResourcesHook_initXResourcesNative as *const () as *mut c_void },
        NativeMethod { name: "makeInheritable".into(), sig: "(Ljava/lang/Class;)Z".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_ResourcesHook_makeInheritable as *const () as *mut c_void },
        NativeMethod { name: "buildDummyClassLoader".into(), sig: "(Ljava/lang/ClassLoader;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/ClassLoader;".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_ResourcesHook_buildDummyClassLoader as *const () as *mut c_void },
        NativeMethod { name: "rewriteXmlReferencesNative".into(), sig: rewrite_signature.as_str().into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_ResourcesHook_rewriteXmlReferencesNative as *const () as *mut c_void },
    ];
    let _ = env.register_native_methods(&class, &methods);
}
