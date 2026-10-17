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

//! Rust part of `liblspd.so`.
//!
//! The native bridge JNI entry point lives here; the rest of the core is
//! still C++ and is called through a small C ABI.

mod atexit;
#[cfg(target_os = "android")]
mod class_loader;
mod elf_util;
mod preloaded_dex;

#[cfg(target_os = "android")]
mod native_api;

#[cfg(target_os = "android")]
mod zygisk_main;
