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
 * Copyright (C) 2019 Swift Gan
 * Copyright (C) 2021 LSPosed Contributors
 */
#ifndef SANDHOOK_ELF_UTIL_H
#define SANDHOOK_ELF_UTIL_H

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <type_traits>
#include <vector>

extern "C" {
void *lspd_elf_create(const char *name, size_t size);
void lspd_elf_destroy(void *handle);
bool lspd_elf_valid(void *handle);
const char *lspd_elf_name(void *handle);
uintptr_t lspd_elf_lookup(void *handle, const char *name, size_t size);
uintptr_t lspd_elf_lookup_prefix(void *handle, const char *prefix, size_t size);
void lspd_elf_lookup_all(void *handle, const char *name, size_t size,
                         void (*callback)(void *, uintptr_t), void *context);
}

namespace SandHook {
    class ElfImg {
    public:
        explicit ElfImg(std::string_view elf);

        // Adopt an existing Rust ElfHandle without taking ownership.
        explicit ElfImg(void *handle) : handle_(handle), owned_(false) {}

        ~ElfImg();

        ElfImg(const ElfImg &) = delete;

        ElfImg &operator=(const ElfImg &) = delete;

        ElfImg(ElfImg &&) = delete;

        ElfImg &operator=(ElfImg &&) = delete;

        template<typename T = void*>
        requires(std::is_pointer_v<T>)
        const T getSymbAddress(std::string_view name) const {
            return reinterpret_cast<T>(lspd_elf_lookup(handle_, name.data(), name.size()));
        }

        template<typename T = void*>
        requires(std::is_pointer_v<T>)
        const T getSymbPrefixFirstAddress(std::string_view prefix) const {
            return reinterpret_cast<T>(
                    lspd_elf_lookup_prefix(handle_, prefix.data(), prefix.size()));
        }

        template<typename T = void*>
        requires(std::is_pointer_v<T>)
        const std::vector<T> getAllSymbAddress(std::string_view name) const {
            std::vector<T> result;
            lspd_elf_lookup_all(handle_, name.data(), name.size(),
                                [](void *context, uintptr_t address) {
                                    static_cast<std::vector<T> *>(context)->push_back(
                                            reinterpret_cast<T>(address));
                                }, &result);
            return result;
        }

        bool isValid() const {
            return lspd_elf_valid(handle_);
        }

        const std::string name() const {
            const char *resolved = lspd_elf_name(handle_);
            return resolved ? std::string(resolved) : std::string();
        }

    private:
        void *handle_ = nullptr;
        bool owned_ = true;
    };

    inline ElfImg::ElfImg(std::string_view elf)
            : handle_(lspd_elf_create(elf.data(), elf.size())) {}

    inline ElfImg::~ElfImg() {
        if (owned_) lspd_elf_destroy(handle_);
    }
}

#endif //SANDHOOK_ELF_UTIL_H
