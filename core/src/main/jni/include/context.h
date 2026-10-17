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

#pragma once

#include <cstddef>
#include <utility>
#include <unistd.h>
#include <vector>
#include <string>
#include <tuple>
#include <string_view>
#include <lsplant.hpp>
#include "utils.h"
#include "utils/jni_helper.hpp"

extern "C" {
void *lspd_preloaded_dex_create(int fd, size_t size);
void lspd_preloaded_dex_destroy(void *handle);
void *lspd_preloaded_dex_data(void *handle);
size_t lspd_preloaded_dex_size(void *handle);
}

namespace lspd {
    class Context {

    public:
        inline static Context *GetInstance() {
            return instance_.get();
        }

        inline static std::unique_ptr<Context> ReleaseInstance() {
            return std::move(instance_);
        }

        inline jobject GetCurrentClassLoader() const { return inject_class_loader_; }

        inline lsplant::ScopedLocalRef<jclass>
        FindClassFromCurrentLoader(JNIEnv *env, std::string_view className) const {
            return FindClassFromLoader(env, GetCurrentClassLoader(), className);
        };

        virtual ~Context() = default;

    protected:
        static std::unique_ptr<Context> instance_;
        jobject inject_class_loader_ = nullptr;
        jclass entry_class_ = nullptr;

        struct PreloadedDex {

            PreloadedDex() = default;

            PreloadedDex(const PreloadedDex &) = delete;

            PreloadedDex &operator=(const PreloadedDex &) = delete;

            PreloadedDex(int fd, std::size_t size)
                    : handle_(lspd_preloaded_dex_create(fd, size)) {}

            PreloadedDex &operator=(PreloadedDex &&other) noexcept {
                if (this != &other) {
                    lspd_preloaded_dex_destroy(handle_);
                    handle_ = other.handle_;
                    other.handle_ = nullptr;
                }
                return *this;
            }

            PreloadedDex(PreloadedDex &&other) noexcept : handle_(other.handle_) {
                other.handle_ = nullptr;
            }

            operator bool() const {
                return lspd_preloaded_dex_size(handle_) != 0;
            }

            auto size() const {
                return lspd_preloaded_dex_size(handle_);
            }

            auto data() const {
                return lspd_preloaded_dex_data(handle_);
            }

            ~PreloadedDex() {
                lspd_preloaded_dex_destroy(handle_);
            }

        private:
            void *handle_ = nullptr;
        };

        Context() {}

        static lsplant::ScopedLocalRef<jclass> FindClassFromLoader(JNIEnv *env, jobject class_loader,
                                                                   std::string_view class_name);

        template<typename ...Args>
        inline void FindAndCall(JNIEnv *env, std::string_view method_name, std::string_view method_sig,
                                Args &&... args) const {
            if (!entry_class_) [[unlikely]] {
                LOGE("cannot call method {}, entry class is null", method_name);
                return;
            }
            jmethodID mid = lsplant::JNI_GetStaticMethodID(env, entry_class_, method_name, method_sig);
            if (mid) [[likely]] {
                env->CallStaticVoidMethod(entry_class_, mid, lsplant::UnwrapScope(std::forward<Args>(args))...);
            } else {
                LOGE("method {} id is null", method_name);
            }
        }

        virtual void InitArtHooker(JNIEnv *env, const lsplant::InitInfo &initInfo);

        virtual void InitHooks(JNIEnv *env);

        virtual void LoadDex(JNIEnv *env, PreloadedDex &&dex) = 0;

        virtual void SetupEntryClass(JNIEnv *env) = 0;

    private:
        friend std::unique_ptr<Context> std::make_unique<Context>();
    };

}
