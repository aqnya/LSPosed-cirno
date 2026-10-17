// This file is part of LSPosed.
//
// LSPosed is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// LSPosed is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with LSPosed.  If not, see <https://www.gnu.org/licenses/>.
//
// Copyright (C) 2022 - 2026 LSPosed Contributors

//! Rust reimplementation of the `dex2oat` wrapper.
//!
//! The wrapper receives the stock dex2oat executable (and the preload shim) over a
//! local abstract socket from the LSPosed daemon, then `fexecve`s the stock binary
//! with `LD_PRELOAD` pointing at the shim and `LD_LIBRARY_PATH` configured.

use std::ffi::{c_char, c_int, c_void, CString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::process;
use std::ptr;

const SOCK_NAME: &[u8] = b"5291374ceda0aef7c5d86cd2a4f6a3ac";
const LOG_TAG: &[u8] = b"LSPosedDex2Oat\0";
const ANDROID_LOG_ERROR: c_int = 6;
const LD_LIBRARY_PATH: &str = "/apex/com.android.art/lib64:/apex/com.android.art/lib:/apex/com.android.os.statsd/lib64:/apex/com.android.os.statsd/lib";

#[link(name = "log")]
extern "C" {
    fn __android_log_print(
        prio: c_int,
        tag: *const c_char,
        fmt: *const c_char,
        ...
    ) -> c_int;
}

extern "C" {
    static mut environ: *mut *mut c_char;
    fn fexecve(fd: c_int, argv: *const *const c_char, envp: *const *const c_char) -> c_int;
}

fn log_error(message: &str) {
    if let Ok(message) = CString::new(message) {
        unsafe {
            __android_log_print(
                ANDROID_LOG_ERROR,
                LOG_TAG.as_ptr().cast(),
                b"%s\0".as_ptr().cast(),
                message.as_ptr(),
            );
        }
    }
}

fn set_env(name: &str, value: &str) {
    let name = CString::new(name).unwrap();
    let value = CString::new(value).unwrap();
    unsafe {
        libc::setenv(name.as_ptr(), value.as_ptr(), 1);
    }
}

fn write_int(fd: c_int, value: i32) -> io::Result<()> {
    let bytes = value.to_ne_bytes();
    let mut written = 0;
    while written < bytes.len() {
        let count = unsafe {
            libc::write(
                fd,
                bytes[written..].as_ptr() as *const c_void,
                bytes.len() - written,
            )
        };
        if count <= 0 {
            return Err(io::Error::last_os_error());
        }
        written += count as usize;
    }
    Ok(())
}

fn read_int(fd: c_int) -> io::Result<i32> {
    let mut bytes = [0u8; 4];
    let mut read = 0;
    while read < bytes.len() {
        let count = unsafe {
            libc::read(
                fd,
                bytes[read..].as_mut_ptr() as *mut c_void,
                bytes.len() - read,
            )
        };
        if count <= 0 {
            return Err(io::Error::last_os_error());
        }
        read += count as usize;
    }
    Ok(i32::from_ne_bytes(bytes))
}

unsafe fn recv_fd(fd: c_int) -> Option<c_int> {
    let mut count: c_int = 1;
    let mut iov = libc::iovec {
        iov_base: &mut count as *mut _ as *mut c_void,
        iov_len: std::mem::size_of::<c_int>(),
    };
    let fd_size = std::mem::size_of::<c_int>() as libc::c_uint;
    let control_len = libc::CMSG_SPACE(fd_size) as usize;
    let mut control = vec![0u8; control_len];
    let mut message: libc::msghdr = std::mem::zeroed();
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr() as *mut c_void;
    message.msg_controllen = control.len();

    if libc::recvmsg(fd, &mut message as *mut _, libc::MSG_WAITALL) < 0 {
        log_error(&format!("recvmsg failed: {}", io::Error::last_os_error()));
        return None;
    }

    let header = libc::CMSG_FIRSTHDR(&message as *const _);
    if header.is_null() {
        return None;
    }
    let header = &*header;
    if message.msg_controllen != control.len()
        || header.cmsg_len != libc::CMSG_LEN(fd_size * count as libc::c_uint) as usize
        || header.cmsg_level != libc::SOL_SOCKET
        || header.cmsg_type != libc::SCM_RIGHTS
    {
        return None;
    }
    Some(*(libc::CMSG_DATA(header as *const _) as *const c_int))
}

fn request_fd(id: i32) -> Option<c_int> {
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
        if fd < 0 {
            log_error(&format!("socket failed: {}", io::Error::last_os_error()));
            return None;
        }

        let mut address: libc::sockaddr_un = std::mem::zeroed();
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        // sun_path[0] stays NUL to select the abstract socket namespace.
        for (index, byte) in SOCK_NAME.iter().enumerate() {
            address.sun_path[index + 1] = *byte as c_char;
        }
        let address_len = std::mem::size_of::<libc::sa_family_t>() + SOCK_NAME.len() + 1;
        if libc::connect(
            fd,
            &address as *const _ as *const libc::sockaddr,
            address_len as libc::socklen_t,
        ) != 0
        {
            log_error(&format!(
                "failed to connect to {}: {}",
                String::from_utf8_lossy(SOCK_NAME),
                io::Error::last_os_error()
            ));
            libc::close(fd);
            return None;
        }

        if let Err(error) = write_int(fd, id) {
            log_error(&format!("write failed: {error}"));
            libc::close(fd);
            return None;
        }
        let received = recv_fd(fd);
        let _ = read_int(fd);
        libc::close(fd);
        received
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let is64 = cfg!(target_pointer_width = "64");
    let is_debug = args
        .first()
        .map(|arg| arg.to_string_lossy().contains("dex2oatd"))
        .unwrap_or(false);
    let id = ((is64 as i32) << 1) | (is_debug as i32);

    let Some(stock_fd) = request_fd(id) else {
        process::exit(1);
    };
    let Some(preload_fd) = request_fd(if is64 { 5 } else { 4 }) else {
        process::exit(1);
    };

    if std::env::var_os("LD_LIBRARY_PATH").is_none() {
        set_env("LD_LIBRARY_PATH", LD_LIBRARY_PATH);
    }
    set_env(
        "LD_PRELOAD",
        &format!("/proc/{}/fd/{}", process::id(), preload_fd),
    );

    let mut argv = Vec::with_capacity(args.len());
    for arg in &args {
        match CString::new(arg.as_bytes()) {
            Ok(arg) => argv.push(arg),
            Err(_) => {
                log_error("argv contains an interior NUL");
                process::exit(2);
            }
        }
    }
    let mut argv_pointers: Vec<*const c_char> = argv.iter().map(|arg| arg.as_ptr()).collect();
    argv_pointers.push(ptr::null());

    unsafe {
        fexecve(
            stock_fd,
            argv_pointers.as_ptr(),
            environ as *const *const c_char,
        );
    }
    log_error(&format!("fexecve failed: {}", io::Error::last_os_error()));
    process::exit(2);
}
