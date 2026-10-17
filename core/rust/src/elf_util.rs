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
 * Copyright (C) 2023 - 2026 LSPosed Contributors
 */

//! Rust replacement for the old `elf_util.cpp` symbol resolver.
//!
//! It locates a loaded module through `/proc/self/maps`, parses the ELF file
//! with goblin, and resolves symbols from `.dynsym`, `.symtab` and the XZ
//! compressed `.gnu_debugdata` section.

use goblin::elf::program_header::PT_LOAD;
use goblin::elf::sym::{STT_FUNC, STT_OBJECT};
use goblin::elf::Elf;
use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, c_void, CString};
use std::io::Cursor;
use std::ptr;
use std::slice;
use std::sync::OnceLock;

#[derive(Default)]
struct Symtab {
    exact: HashMap<String, u64>,
    sorted: BTreeMap<String, u64>,
}

impl Symtab {
    fn is_empty(&self) -> bool {
        self.exact.is_empty()
    }
}

struct ElfHandle {
    path: CString,
    base: usize,
    bias: i64,
    dynsyms: HashMap<String, u64>,
    file_data: Vec<u8>,
    debugdata: Option<Vec<u8>>,
    symtab: OnceLock<Symtab>,
}

impl ElfHandle {
    fn new(requested: &str) -> Self {
        let (base, path) = find_module(requested).unwrap_or((0, String::new()));
        let path_c = CString::new(path.as_str()).unwrap_or_default();
        let file_data = std::fs::read(&path).unwrap_or_default();

        let (bias, dynsyms, debugdata) = if file_data.is_empty() {
            (0, HashMap::new(), None)
        } else {
            parse_elf(&file_data)
        };

        Self {
            path: path_c,
            base,
            bias,
            dynsyms,
            file_data,
            debugdata,
            symtab: OnceLock::new(),
        }
    }

    fn valid(&self) -> bool {
        self.base != 0
    }

    fn address(&self, value: u64) -> usize {
        if value == 0 || self.base == 0 {
            return 0;
        }
        let address = self.base as i64 + value as i64 - self.bias;
        if address <= 0 {
            0
        } else {
            address as usize
        }
    }

    fn symtab(&self) -> &Symtab {
        self.symtab.get_or_init(|| {
            if let Some(debugdata) = self.debugdata.as_deref() {
                let inner = collect_symtab(debugdata);
                if !inner.is_empty() {
                    return inner;
                }
            }
            collect_symtab(&self.file_data)
        })
    }

    fn lookup(&self, name: &str) -> usize {
        if let Some(value) = self.dynsyms.get(name) {
            let address = self.address(*value);
            if address != 0 {
                return address;
            }
        }
        if let Some(value) = self.symtab().exact.get(name) {
            return self.address(*value);
        }
        0
    }

    fn lookup_prefix(&self, prefix: &str) -> usize {
        let symtab = self.symtab();
        if let Some((name, value)) = symtab.sorted.range(prefix.to_string()..).next() {
            if name.starts_with(prefix) {
                return self.address(*value);
            }
        }
        0
    }

    fn lookup_all(&self, name: &str, callback: unsafe extern "C" fn(*mut c_void, usize), context: *mut c_void) {
        if let Some(value) = self.symtab().exact.get(name) {
            let address = self.address(*value);
            if address != 0 {
                unsafe { callback(context, address) };
            }
        }
    }
}

fn find_module(requested: &str) -> Option<(usize, String)> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    for line in maps.lines() {
        if !line.contains("r-xp") && !line.contains("r--p") {
            continue;
        }
        if !line.contains(requested) {
            continue;
        }
        let start = line.split('-').next()?;
        let base = usize::from_str_radix(start, 16).ok()?;
        let path = line.rsplit_once(' ')?.1;
        if !path.starts_with('/') {
            continue;
        }
        return Some((base, path.to_string()));
    }
    None
}

fn parse_elf(data: &[u8]) -> (i64, HashMap<String, u64>, Option<Vec<u8>>) {
    let Ok(elf) = Elf::parse(data) else {
        return (0, HashMap::new(), None);
    };

    let bias = elf
        .program_headers
        .iter()
        .find(|header| header.p_type == PT_LOAD)
        .map(|header| header.p_vaddr as i64 - header.p_offset as i64)
        .unwrap_or(0);

    let mut dynsyms = HashMap::new();
    for symbol in elf.dynsyms.iter() {
        if symbol.st_value == 0 {
            continue;
        }
        if let Some(name) = elf.dynstrtab.get_at(symbol.st_name) {
            if !name.is_empty() {
                dynsyms.entry(name.to_string()).or_insert(symbol.st_value);
            }
        }
    }

    let debugdata = elf
        .section_headers
        .iter()
        .find(|section| elf.shdr_strtab.get_at(section.sh_name) == Some(".gnu_debugdata"))
        .and_then(|section| {
            let start = section.sh_offset as usize;
            let end = start.checked_add(section.sh_size as usize)?;
            let compressed = data.get(start..end)?;
            let mut output = Vec::new();
            lzma_rs::xz_decompress(&mut Cursor::new(compressed), &mut output).ok()?;
            if output.starts_with(b"\x7fELF") {
                Some(output)
            } else {
                None
            }
        });

    (bias, dynsyms, debugdata)
}

fn collect_symtab(data: &[u8]) -> Symtab {
    let mut symtab = Symtab::default();
    let Ok(elf) = Elf::parse(data) else {
        return symtab;
    };

    for symbol in elf.syms.iter() {
        if symbol.st_size == 0 || symbol.st_value == 0 {
            continue;
        }
        let symbol_type = symbol.st_type();
        if symbol_type != STT_FUNC && symbol_type != STT_OBJECT {
            continue;
        }
        let Some(name) = elf.strtab.get_at(symbol.st_name) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        symtab
            .exact
            .entry(name.to_string())
            .or_insert(symbol.st_value);
        symtab
            .sorted
            .entry(name.to_string())
            .or_insert(symbol.st_value);
    }

    symtab
}

unsafe fn handle_ref<'a>(handle: *mut c_void) -> Option<&'a ElfHandle> {
    if handle.is_null() {
        None
    } else {
        Some(&*(handle as *const ElfHandle))
    }
}

unsafe fn bytes_to_str<'a>(data: *const c_char, len: usize) -> Option<&'a str> {
    if data.is_null() {
        return None;
    }
    std::str::from_utf8(slice::from_raw_parts(data as *const u8, len)).ok()
}

#[no_mangle]
pub unsafe extern "C" fn lspd_elf_create(name: *const c_char, len: usize) -> *mut c_void {
    let Some(name) = bytes_to_str(name, len) else {
        return ptr::null_mut();
    };
    Box::into_raw(Box::new(ElfHandle::new(name))) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn lspd_elf_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        drop(Box::from_raw(handle as *mut ElfHandle));
    }
}

#[no_mangle]
pub unsafe extern "C" fn lspd_elf_valid(handle: *mut c_void) -> bool {
    handle_ref(handle).is_some_and(ElfHandle::valid)
}

#[no_mangle]
pub unsafe extern "C" fn lspd_elf_name(handle: *mut c_void) -> *const c_char {
    handle_ref(handle)
        .map(|handle| handle.path.as_ptr())
        .unwrap_or(ptr::null())
}

#[no_mangle]
pub unsafe extern "C" fn lspd_elf_lookup(
    handle: *mut c_void,
    name: *const c_char,
    len: usize,
) -> usize {
    let Some(handle) = handle_ref(handle) else {
        return 0;
    };
    let Some(name) = bytes_to_str(name, len) else {
        return 0;
    };
    handle.lookup(name)
}

#[no_mangle]
pub unsafe extern "C" fn lspd_elf_lookup_prefix(
    handle: *mut c_void,
    prefix: *const c_char,
    len: usize,
) -> usize {
    let Some(handle) = handle_ref(handle) else {
        return 0;
    };
    let Some(prefix) = bytes_to_str(prefix, len) else {
        return 0;
    };
    handle.lookup_prefix(prefix)
}

#[no_mangle]
pub unsafe extern "C" fn lspd_elf_lookup_all(
    handle: *mut c_void,
    name: *const c_char,
    len: usize,
    callback: Option<unsafe extern "C" fn(*mut c_void, usize)>,
    context: *mut c_void,
) {
    let Some(handle) = handle_ref(handle) else {
        return;
    };
    let Some(name) = bytes_to_str(name, len) else {
        return;
    };
    let Some(callback) = callback else {
        return;
    };
    handle.lookup_all(name, callback, context);
}


#[cfg(test)]
mod tests {
    use super::*;

    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    #[test]
    fn resolves_libc_symbols() {
        let name = b"libc.so.6";
        let handle = unsafe { lspd_elf_create(name.as_ptr().cast(), name.len()) };
        assert!(!handle.is_null());
        assert!(unsafe { lspd_elf_valid(handle) });
        for symbol in [b"malloc".as_slice(), b"free".as_slice(), b"printf".as_slice()] {
            let address = unsafe { lspd_elf_lookup(handle, symbol.as_ptr().cast(), symbol.len()) };
            assert_ne!(address, 0, "{}", String::from_utf8_lossy(symbol));
        }
        let name = b"printf\0";
        let ours = unsafe { lspd_elf_lookup(handle, b"printf".as_ptr().cast(), 6) };
        let expected = unsafe { dlsym(ptr::null_mut(), name.as_ptr().cast()) } as usize;
        assert_eq!(ours, expected);
        unsafe { lspd_elf_destroy(handle) };
    }
}
