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
 * Copyright (C) 2021 - 2022 LSPosed Contributors
 */

//
// Created by 双草酸酯 on 2/7/21.
//
#include "native_api.h"
#include "native_util.h"
#include "utils/jni_helper.hpp"
#include "../native_api.h"

#include <jni.h>

using namespace lsplant;

#ifdef LSPD_RUST_CORE
// Implemented in Rust (core/rust/src/lib.rs).
extern "C" void Java_org_lsposed_lspd_nativebridge_NativeAPI_recordNativeEntrypoint(
        JNIEnv *env, jclass clazz, jstring jstr);
#else
extern "C" void Java_org_lsposed_lspd_nativebridge_NativeAPI_recordNativeEntrypoint(
        JNIEnv *env, [[maybe_unused]] jclass clazz, jstring jstr) {
    JUTFString str(env, jstr);
    lspd::RegisterNativeLib(str);
}
#endif

namespace lspd {
    static JNINativeMethod gMethods[] = {
            LSP_NATIVE_METHOD(NativeAPI, recordNativeEntrypoint, "(Ljava/lang/String;)V")
    };

    void RegisterNativeAPI(JNIEnv *env) {
        REGISTER_LSP_NATIVE_METHODS(NativeAPI);
    }
}
