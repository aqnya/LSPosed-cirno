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

#include <jni.h>
#include <sys/types.h>

#include "../api/zygisk.h"
#include "config_impl.h"
#include "magisk_loader.h"

extern "C" void lspd_zygisk_entry(void *table, JNIEnv *env);

// Keep the exported Zygisk ABI symbol in C++ so the linker always pulls the
// Rust object that implements it out of the static archive.
extern "C" [[gnu::visibility("default")]] void zygisk_module_entry(
        zygisk::internal::api_table *table, JNIEnv *env) {
    lspd_zygisk_entry(table, env);
}

extern "C" void lspd_magisk_loader_init() {
    lspd::MagiskLoader::Init();
}

extern "C" void lspd_config_init() {
    lspd::ConfigImpl::Init();
}

extern "C" void lspd_pre_app(JNIEnv *env, jint uid, jintArray *gids,
                             jstring *nice_name, jboolean is_child_zygote,
                             jstring app_data_dir) {
    lspd::MagiskLoader::GetInstance()->OnNativeForkAndSpecializePre(
            env, uid, *gids, *nice_name, is_child_zygote, app_data_dir);
}

extern "C" void lspd_post_app(JNIEnv *env, jstring nice_name, jstring app_data_dir) {
    lspd::MagiskLoader::GetInstance()->OnNativeForkAndSpecializePost(
            env, nice_name, app_data_dir);
}

extern "C" void lspd_pre_server(JNIEnv *env) {
    lspd::MagiskLoader::GetInstance()->OnNativeForkSystemServerPre(env);
}

extern "C" void lspd_post_server(JNIEnv *env) {
    lspd::MagiskLoader::GetInstance()->OnNativeForkSystemServerPost(env);
}
