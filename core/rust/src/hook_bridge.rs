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

//! Rust rewrite of the HookBridge JNI layer. LSPlant and the nonvirtual
//! invocation helpers stay behind a small C ABI.

use jni::objects::{GlobalRef, JClass, JObject, JObjectArray, JValue};
use jni::sys::{
    jboolean, jclass, jint, jmethodID, jobject, jobjectArray, JNIEnv as RawJNIEnv, JNI_FALSE,
    JNI_TRUE,
};
use jni::{JNIEnv, NativeMethod};
use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::{Mutex, OnceLock};

struct HookItem {
    backup: usize,
    callbacks: BTreeMap<jint, Vec<GlobalRef>>,
}

static HOOKS: OnceLock<Mutex<HashMap<usize, HookItem>>> = OnceLock::new();

fn hooks() -> &'static Mutex<HashMap<usize, HookItem>> {
    HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
}

extern "C" {
    fn lspd_lsplant_hook(
        env: *mut RawJNIEnv,
        method: jobject,
        hooker: jobject,
        callback: jobject,
    ) -> jobject;
    fn lspd_lsplant_deoptimize(env: *mut RawJNIEnv, method: jobject) -> jboolean;
    fn lspd_make_dex_file_trusted(env: *mut RawJNIEnv, cookie: jobject) -> jboolean;
    fn lspd_hook_bridge_invoke_original(
        env: *mut RawJNIEnv,
        method: jobject,
        receiver: jobject,
        args: jobjectArray,
        is_constructor: jboolean,
        backup: jobject,
    ) -> jobject;
    fn lspd_hook_bridge_allocate_special(
        env: *mut RawJNIEnv,
        constructor: jobject,
        class: jclass,
    ) -> jobject;
    fn lspd_hook_bridge_invoke_special(
        env: *mut RawJNIEnv,
        method: jobject,
        class: jclass,
        receiver: jobject,
        args: jobjectArray,
    ) -> jobject;
    fn lspd_get_current_class_loader() -> jobject;
    fn lspd_native_bridge_prefix(length: *mut usize) -> *const c_char;
}

unsafe fn from_reflected_method(env: &JNIEnv, method: &JObject) -> Option<jmethodID> {
    let raw = env.get_native_interface();
    let functions = *raw;
    let Some(from_reflected) = (*functions).FromReflectedMethod else {
        return None;
    };
    let id = from_reflected(raw, method.as_raw());
    if id.is_null() {
        None
    } else {
        Some(id)
    }
}

unsafe fn to_reflected_method<'local>(
    env: &JNIEnv<'local>,
    class: &JClass<'local>,
    id: jmethodID,
    is_static: bool,
) -> Option<JObject<'local>> {
    let raw = env.get_native_interface();
    let functions = *raw;
    let Some(to_reflected) = (*functions).ToReflectedMethod else {
        return None;
    };
    let object = to_reflected(raw, class.as_raw(), id, is_static as jboolean);
    if object.is_null() {
        None
    } else {
        Some(JObject::from_raw(object))
    }
}

fn add_callback(item: &mut HookItem, env: &mut JNIEnv, priority: jint, callback: &JObject) {
    if let Ok(callback) = env.new_global_ref(callback) {
        item.callbacks.entry(priority).or_default().push(callback);
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_hookMethod(
    mut env: JNIEnv,
    _class: JClass,
    method: JObject,
    hooker: JClass,
    priority: jint,
    callback: JObject,
) -> jboolean {
    if method.is_null() || callback.is_null() {
        return JNI_FALSE;
    }
    let target = unsafe {
        match from_reflected_method(&env, &method) {
            Some(target) => target as usize,
            None => return JNI_FALSE,
        }
    };

    let mut hooks = match hooks().lock() {
        Ok(hooks) => hooks,
        Err(_) => return JNI_FALSE,
    };
    if let Some(item) = hooks.get_mut(&target) {
        add_callback(item, &mut env, priority, &callback);
        return JNI_TRUE;
    }

    let hooker_object = match env.new_object(
        &hooker,
        "(Ljava/lang/reflect/Executable;)V",
        &[JValue::Object(&method)],
    ) {
        Ok(object) => object,
        Err(_) => return JNI_FALSE,
    };
    let callback_method_id = match env.get_method_id(
        &hooker,
        "callback",
        "([Ljava/lang/Object;)Ljava/lang/Object;",
    ) {
        Ok(id) => id,
        Err(_) => return JNI_FALSE,
    };
    let callback_method = match unsafe {
        to_reflected_method(&env, &hooker, callback_method_id.into_raw(), false)
    } {
        Some(method) => method,
        None => return JNI_FALSE,
    };

    let backup = unsafe {
        lspd_lsplant_hook(
            env.get_native_interface(),
            method.as_raw(),
            hooker_object.as_raw(),
            callback_method.as_raw(),
        )
    };
    if backup.is_null() {
        return JNI_FALSE;
    }

    let mut item = HookItem {
        backup: backup as usize,
        callbacks: BTreeMap::new(),
    };
    add_callback(&mut item, &mut env, priority, &callback);
    hooks.insert(target, item);
    JNI_TRUE
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_unhookMethod(
    mut env: JNIEnv,
    _class: JClass,
    method: JObject,
    callback: JObject,
) -> jboolean {
    if method.is_null() || callback.is_null() {
        return JNI_FALSE;
    }
    let target = unsafe {
        match from_reflected_method(&env, &method) {
            Some(target) => target as usize,
            None => return JNI_FALSE,
        }
    };
    let mut hooks = match hooks().lock() {
        Ok(hooks) => hooks,
        Err(_) => return JNI_FALSE,
    };
    let Some(item) = hooks.get_mut(&target) else {
        return JNI_FALSE;
    };
    let mut removed = false;
    for callbacks in item.callbacks.values_mut() {
        let position = callbacks
            .iter()
            .position(|saved| env.is_same_object(saved.as_obj(), &callback).unwrap_or(false));
        if let Some(position) = position {
            callbacks.remove(position);
            removed = true;
            break;
        }
    }
    if removed {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_deoptimizeMethod(
    mut env: JNIEnv,
    _class: JClass,
    method: JObject,
) -> jboolean {
    if method.is_null() {
        return JNI_FALSE;
    }
    unsafe { lspd_lsplant_deoptimize(env.get_native_interface(), method.as_raw()) }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_invokeOriginalMethod(
    mut env: JNIEnv,
    _class: JClass,
    method: JObject,
    receiver: JObject,
    args: JObjectArray,
    is_constructor: jboolean,
) -> jobject {
    if method.is_null() {
        return ptr::null_mut();
    }
    let target = unsafe {
        match from_reflected_method(&env, &method) {
            Some(target) => target as usize,
            None => return ptr::null_mut(),
        }
    };
    let backup = {
        let hooks = match hooks().lock() {
            Ok(hooks) => hooks,
            Err(_) => return ptr::null_mut(),
        };
        hooks
            .get(&target)
            .map(|item| item.backup)
            .unwrap_or(0)
    };
    unsafe {
        lspd_hook_bridge_invoke_original(
            env.get_native_interface(),
            method.as_raw(),
            receiver.as_raw(),
            args.as_raw(),
            is_constructor,
            backup as jobject,
        )
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_findClassInitializer(
    mut env: JNIEnv,
    _class: JClass,
    class: JClass,
) -> jobject {
    if class.is_null() {
        return ptr::null_mut();
    }
    let id = match env.get_static_method_id(&class, "<clinit>", "()V") {
        Ok(id) => id,
        Err(_) => {
            let _ = env.exception_clear();
            return ptr::null_mut();
        }
    };
    match unsafe { to_reflected_method(&env, &class, id.into_raw(), true) } {
        Some(method) => method.into_raw(),
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_allocateObject(
    mut env: JNIEnv,
    _class: JClass,
    class: JClass,
) -> jobject {
    if class.is_null() {
        return ptr::null_mut();
    }
    match env.alloc_object(&class) {
        Ok(object) => object.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_allocateSpecialReceiver(
    mut env: JNIEnv,
    _class: JClass,
    constructor: JObject,
    class: JClass,
) -> jobject {
    if constructor.is_null() || class.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        lspd_hook_bridge_allocate_special(
            env.get_native_interface(),
            constructor.as_raw(),
            class.as_raw(),
        )
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_invokeSpecialMethod(
    mut env: JNIEnv,
    _class: JClass,
    method: JObject,
    class: JClass,
    receiver: JObject,
    args: JObjectArray,
) -> jobject {
    if method.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        lspd_hook_bridge_invoke_special(
            env.get_native_interface(),
            method.as_raw(),
            class.as_raw(),
            receiver.as_raw(),
            args.as_raw(),
        )
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_instanceOf(
    mut env: JNIEnv,
    _class: JClass,
    object: JObject,
    expected_class: JClass,
) -> jboolean {
    match env.is_instance_of(&object, &expected_class) {
        Ok(true) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_gettid(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    unsafe { libc::gettid() as jint }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_setTrusted(
    mut env: JNIEnv,
    _class: JClass,
    cookie: JObject,
) -> jboolean {
    if cookie.is_null() {
        return JNI_FALSE;
    }
    unsafe { lspd_make_dex_file_trusted(env.get_native_interface(), cookie.as_raw()) }
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_nativebridge_HookBridge_callbackSnapshot(
    mut env: JNIEnv,
    _class: JClass,
    method: JObject,
    max_priority: jint,
) -> jobjectArray {
    if method.is_null() {
        return ptr::null_mut();
    }
    let target = unsafe {
        match from_reflected_method(&env, &method) {
            Some(target) => target as usize,
            None => return ptr::null_mut(),
        }
    };
    let hooks = match hooks().lock() {
        Ok(hooks) => hooks,
        Err(_) => return ptr::null_mut(),
    };
    let Some(item) = hooks.get(&target) else {
        return empty_object_array(&mut env);
    };
    let count: usize = item
        .callbacks
        .iter()
        .rev()
        .filter(|(priority, _)| **priority <= max_priority)
        .map(|(_, callbacks)| callbacks.len())
        .sum();
    let array = match env.new_object_array(count as jint, "java/lang/Object", JObject::null()) {
        Ok(array) => array,
        Err(_) => return ptr::null_mut(),
    };
    let mut index = 0;
    for (priority, callbacks) in item.callbacks.iter().rev() {
        if *priority > max_priority {
            continue;
        }
        for callback in callbacks {
            let _ = env.set_object_array_element(&array, index, callback.as_obj());
            index += 1;
        }
    }
    array.into_raw()
}

fn empty_object_array(env: &mut JNIEnv) -> jobjectArray {
    env.new_object_array(0, "java/lang/Object", JObject::null())
        .map(|array| array.into_raw())
        .unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn lspd_register_hook_bridge(raw_env: *mut RawJNIEnv) {
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
    let mut class_name = Vec::with_capacity(prefix.len() + 10);
    class_name.extend_from_slice(prefix);
    class_name.extend_from_slice(b"HookBridge");
    let class = crate::class_loader::lspd_find_class_from_loader(
        raw_env,
        class_loader,
        class_name.as_ptr().cast(),
        class_name.len(),
    );
    if class.is_null() {
        return;
    }
    let class = JClass::from_raw(class);

    let methods = [
        NativeMethod { name: "hookMethod".into(), sig: "(Ljava/lang/reflect/Executable;Ljava/lang/Class;ILjava/lang/Object;)Z".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_hookMethod as *const () as *mut c_void },
        NativeMethod { name: "unhookMethod".into(), sig: "(Ljava/lang/reflect/Executable;Ljava/lang/Object;)Z".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_unhookMethod as *const () as *mut c_void },
        NativeMethod { name: "deoptimizeMethod".into(), sig: "(Ljava/lang/reflect/Executable;)Z".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_deoptimizeMethod as *const () as *mut c_void },
        NativeMethod { name: "invokeOriginalMethod".into(), sig: "(Ljava/lang/reflect/Executable;Ljava/lang/Object;[Ljava/lang/Object;Z)Ljava/lang/Object;".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_invokeOriginalMethod as *const () as *mut c_void },
        NativeMethod { name: "findClassInitializer".into(), sig: "(Ljava/lang/Class;)Ljava/lang/reflect/Method;".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_findClassInitializer as *const () as *mut c_void },
        NativeMethod { name: "allocateObject".into(), sig: "(Ljava/lang/Class;)Ljava/lang/Object;".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_allocateObject as *const () as *mut c_void },
        NativeMethod { name: "allocateSpecialReceiver".into(), sig: "(Ljava/lang/reflect/Constructor;Ljava/lang/Class;)Ljava/lang/Object;".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_allocateSpecialReceiver as *const () as *mut c_void },
        NativeMethod { name: "invokeSpecialMethod".into(), sig: "(Ljava/lang/reflect/Executable;Ljava/lang/Class;Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_invokeSpecialMethod as *const () as *mut c_void },
        NativeMethod { name: "instanceOf".into(), sig: "(Ljava/lang/Object;Ljava/lang/Class;)Z".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_instanceOf as *const () as *mut c_void },
        NativeMethod { name: "gettid".into(), sig: "()I".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_gettid as *const () as *mut c_void },
        NativeMethod { name: "setTrusted".into(), sig: "(Ljava/lang/Object;)Z".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_setTrusted as *const () as *mut c_void },
        NativeMethod { name: "callbackSnapshot".into(), sig: "(Ljava/lang/reflect/Executable;I)[Ljava/lang/Object;".into(), fn_ptr: Java_org_lsposed_lspd_nativebridge_HookBridge_callbackSnapshot as *const () as *mut c_void },
    ];
    let _ = env.register_native_methods(&class, &methods);
}
