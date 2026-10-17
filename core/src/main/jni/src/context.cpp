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

#include <jni.h>

#include "config.h"
#include "context.h"
#include "native_util.h"
#include "symbol_cache.h"

using namespace lsplant;

extern "C" jclass lspd_find_class_from_loader(JNIEnv *env, jobject class_loader,
                                               const char *class_name, size_t class_name_len);
extern "C" void lspd_init_hooks(JNIEnv *env, jobject class_loader);
extern "C" jboolean lspd_make_dex_file_trusted(JNIEnv *env, jobject cookie);
extern "C" void lspd_register_hook_bridge(JNIEnv *env);


namespace lspd {
    std::unique_ptr<Context> Context::instance_;
    std::unique_ptr<ConfigBridge> ConfigBridge::instance_;

    void Context::InitArtHooker(JNIEnv *env, const lsplant::InitInfo &initInfo) {
        if (!lsplant::Init(env, initInfo)) {
            LOGE("Failed to init lsplant");
            return;
        }
    }

    void Context::InitHooks(JNIEnv *env) {
        lspd_init_hooks(env, inject_class_loader_);
    }

    ScopedLocalRef<jclass>
    Context::FindClassFromLoader(JNIEnv *env, jobject class_loader,
                                 std::string_view class_name) {
        auto clazz = reinterpret_cast<jclass>(lspd_find_class_from_loader(
                env, class_loader, class_name.data(), class_name.size()));
        if (!clazz) {
            LOGE("Class {} not found", class_name);
        }
        return {env, clazz};
    }
}  // namespace lspd

extern "C" jboolean lspd_make_dex_file_trusted(JNIEnv *env, jobject cookie) {
    return lsplant::MakeDexFileTrusted(env, cookie);
}

extern "C" jobject lspd_get_current_class_loader() {
    auto *context = lspd::Context::GetInstance();
    return context ? context->GetCurrentClassLoader() : nullptr;
}

extern "C" const char *lspd_native_bridge_prefix(size_t *length) {
    static auto signature = lspd::GetNativeBridgeSignature();
    if (length) *length = signature.size();
    return signature.c_str();
}
