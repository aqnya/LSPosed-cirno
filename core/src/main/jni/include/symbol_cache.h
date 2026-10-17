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
 * Copyright (C) 2021 LSPosed Contributors
 */

//
// Created by kotori on 2/7/21.
//

#ifndef LSPOSED_SYMBOL_CACHE_H
#define LSPOSED_SYMBOL_CACHE_H

#include <memory>
#include "elf_util.h"

extern "C" {
void *lspd_symbol_cache_get_art();
void lspd_symbol_cache_release_art();
void *lspd_symbol_cache_get_binder();
void lspd_symbol_cache_release_binder();
}

namespace lspd {
    inline std::unique_ptr<const SandHook::ElfImg> &GetArt(bool release = false) {
        static std::unique_ptr<const SandHook::ElfImg> image = nullptr;
        if (release) {
            image.reset();
            lspd_symbol_cache_release_art();
        } else if (!image) {
            image = std::make_unique<const SandHook::ElfImg>(lspd_symbol_cache_get_art());
        }
        return image;
    }

    inline std::unique_ptr<const SandHook::ElfImg> &GetLibBinder(bool release = false) {
        static std::unique_ptr<const SandHook::ElfImg> image = nullptr;
        if (release) {
            image.reset();
            lspd_symbol_cache_release_binder();
        } else if (!image) {
            image = std::make_unique<const SandHook::ElfImg>(lspd_symbol_cache_get_binder());
        }
        return image;
    }
}

#endif //LSPOSED_SYMBOL_CACHE_H
