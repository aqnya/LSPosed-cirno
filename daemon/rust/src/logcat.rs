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

//! Rust takeover of `daemon/src/main/jni/logcat.cpp`: reads the Android log
//! buffers, filters LSPosed-relevant entries, rotates log files and maintains
//! the logd settings watchdog.

use jni::objects::{JObject, JValue};
use jni::JNIEnv;
use std::ffi::{c_char, c_int, c_long, c_uint, c_void, CStr, CString};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::FromRawFd;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const LOGGER_ENTRY_MAX_LEN: usize = 5 * 1024;
const MAX_LOG_SIZE: usize = 4 * 1024 * 1024;
const LOG_BUFFER_SIZE: usize = 64 * 1024;
const PROP_VALUE_MAX: usize = 92;
const NS_PER_SEC: c_long = 1_000_000_000;
const MS_PER_NSEC: c_long = 1_000_000;

const LOG_ID_MAIN: c_int = 0;
const LOG_ID_CRASH: c_int = 4;

const LOG_CHARS: [u8; 9] = [b'?', b'?', b'V', b'D', b'I', b'W', b'E', b'F', b'S'];

const PROP_CTL_RESTART: &[u8] = b"ctl.restart\0";
const PROP_CTL_START: &[u8] = b"ctl.start\0";
const PROP_LOGD_SIZE: &[u8] = b"persist.logd.size\0";
const PROP_LOGD_TAG: &[u8] = b"persist.log.tag\0";
const PROP_LOGD_MAIN_SIZE: &[u8] = b"persist.logd.size.main\0";
const PROP_LOGD_CRASH_SIZE: &[u8] = b"persist.logd.size.crash\0";

const TAG_BRIDGE: &[u8] = b"LSPosed-Bridge";
const TAG_XSHARED: &[u8] = b"XSharedPreferences";
const TAG_CONTEXT: &[u8] = b"LSPosedContext";
const TAG_LOGCAT: &[u8] = b"LSPosedLogcat";
const TAG_MAGISK: &[u8] = b"Magisk";
const TAG_DOBBY: &[u8] = b"Dobby";
const TAG_LSPLANT: &[u8] = b"LSPlant";
const TAG_ZYGISK: &[u8] = b"zygisk";
const TAG_LSPOSED: &[u8] = b"LSPosed";

const MSG_START_VERBOSE: &[u8] = b"!!start_verbose!!";
const MSG_STOP_VERBOSE: &[u8] = b"!!stop_verbose!!";
const MSG_REFRESH_MODULES: &[u8] = b"!!refresh_modules!!";
const MSG_REFRESH_VERBOSE: &[u8] = b"!!refresh_verbose!!";

#[repr(C)]
#[derive(Clone, Copy)]
struct LoggerEntry {
    len: u16,
    hdr_size: u16,
    pid: i32,
    tid: u32,
    sec: u32,
    nsec: u32,
    lid: u32,
    uid: u32,
}

#[repr(C, align(4))]
struct LogMsg([u8; LOGGER_ENTRY_MAX_LEN + 1]);

#[repr(C)]
struct AndroidLogEntry {
    tv_sec: libc::time_t,
    tv_nsec: c_long,
    priority: c_int,
    uid: i32,
    pid: i32,
    tid: i32,
    tag: *const c_char,
    tag_len: usize,
    message_len: usize,
    message: *const c_char,
}

#[repr(C)]
struct LoggerList {
    _private: [u8; 0],
}

#[repr(C)]
struct Logger {
    _private: [u8; 0],
}

#[repr(C)]
struct PropInfo {
    _private: [u8; 0],
}

#[link(name = "log")]
extern "C" {
    fn android_logger_list_alloc(mode: c_int, tail: c_uint, pid: libc::pid_t) -> *mut LoggerList;
    fn android_logger_list_free(list: *mut LoggerList);
    fn android_logger_list_read(list: *mut LoggerList, msg: *mut c_void) -> c_int;
    fn android_logger_open(list: *mut LoggerList, id: c_int) -> *mut Logger;
    fn android_logger_get_log_size(logger: *mut Logger) -> c_long;
    fn android_logger_set_log_size(logger: *mut Logger, size: libc::c_ulong) -> c_int;
    fn android_log_processLogBuffer(entry: *const LoggerEntry, out: *mut AndroidLogEntry) -> c_int;
}

extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> c_int;
    fn __system_property_set(name: *const c_char, value: *const c_char) -> c_int;
    fn __system_property_find(name: *const c_char) -> *const PropInfo;
    fn __system_property_read_callback(
        pi: *const PropInfo,
        callback: extern "C" fn(*mut c_void, *const c_char, *const c_char, u32),
        cookie: *mut c_void,
    );
    fn __system_property_wait(
        pi: *const PropInfo,
        old_serial: u32,
        new_serial: *mut u32,
        timeout: *const libc::timespec,
    ) -> bool;
}

fn parse_uint(value: &str) -> Option<usize> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('-') {
        return None;
    }
    let (radix, digits) = if let Some(rest) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        (16, rest)
    } else {
        (10, value)
    };
    let mut end = 0;
    for (index, byte) in digits.bytes().enumerate() {
        let valid = if radix == 16 {
            byte.is_ascii_hexdigit()
        } else {
            byte.is_ascii_digit()
        };
        if !valid {
            break;
        }
        end = index + 1;
    }
    if end == 0 {
        return None;
    }
    let mut result = u64::from_str_radix(&digits[..end], radix).ok()?;
    let suffix = digits[end..].chars().next();
    if let Some(suffix) = suffix {
        let shift = match suffix.to_ascii_lowercase() {
            'b' => 0,
            'k' => 10,
            'm' => 20,
            'g' => 30,
            't' => 40,
            'p' => 50,
            'e' => 60,
            _ => return None,
        };
        result = result.checked_mul(1u64 << shift)?;
    }
    usize::try_from(result).ok()
}

fn get_byte_prop(name: &[u8]) -> Option<usize> {
    let mut buffer = [0 as c_char; PROP_VALUE_MAX];
    if unsafe { __system_property_get(name.as_ptr().cast(), buffer.as_mut_ptr()) } < 0 {
        return None;
    }
    let value = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
    parse_uint(&value)
}

fn prop_exists(name: &[u8]) -> bool {
    !unsafe { __system_property_find(name.as_ptr().cast()) }.is_null()
}

fn set_int_prop(name: &[u8], value: usize) {
    if let Ok(value) = CString::new(value.to_string()) {
        unsafe { __system_property_set(name.as_ptr().cast(), value.as_ptr()) };
    }
}

fn set_str_prop(name: &[u8], value: &[u8]) {
    if let Ok(value) = CString::new(value) {
        unsafe { __system_property_set(name.as_ptr().cast(), value.as_ptr()) };
    }
}

struct Output {
    file: Option<File>,
}

impl Output {
    fn from_fd(fd: c_int) -> Self {
        Self {
            file: if fd > 0 {
                Some(unsafe { File::from_raw_fd(fd) })
            } else {
                None
            },
        }
    }

    fn write_all(&mut self, data: &[u8]) {
        match &mut self.file {
            Some(file) => {
                let _ = file.write_all(data);
            }
            None => {
                let _ = io::stdout().write_all(data);
            }
        }
    }

    fn flush(&mut self) {
        match &mut self.file {
            Some(file) => {
                let _ = file.flush();
            }
            None => {
                let _ = io::stdout().flush();
            }
        }
    }
}

struct Files {
    verbose: Output,
    verbose_part: usize,
    verbose_count: usize,
    modules: Output,
    modules_part: usize,
    modules_count: usize,
    verbose_enabled: bool,
}

impl Files {
    fn new() -> Self {
        Self {
            verbose: Output::from_fd(-1),
            verbose_part: 0,
            verbose_count: 0,
            modules: Output::from_fd(-1),
            modules_part: 0,
            modules_count: 0,
            verbose_enabled: true,
        }
    }

    fn write_verbose(&mut self, data: &[u8]) {
        self.verbose.write_all(data);
        self.verbose.flush();
        self.verbose_count += data.len();
    }

    fn write_modules(&mut self, data: &[u8]) {
        self.modules.write_all(data);
        self.modules.flush();
        self.modules_count += data.len();
    }

    fn log(&mut self, data: &[u8]) {
        if self.verbose_enabled {
            self.write_verbose(data);
        }
        self.write_modules(data);
    }
}

struct Logcat<'local> {
    env: JNIEnv<'local>,
    thiz: JObject<'local>,
    files: Arc<Mutex<Files>>,
    my_pid: libc::pid_t,
}

impl<'local> Logcat<'local> {
    fn refresh_fd(&mut self, verbose: bool) -> c_int {
        let result = (|| -> jni::errors::Result<c_int> {
            let class = self.env.get_object_class(&self.thiz)?;
            let method = self.env.get_method_id(&class, "refreshFd", "(Z)I")?;
            let value = self.env.call_method(
                &self.thiz,
                method,
                &[JValue::Bool(if verbose { 1 } else { 0 })],
            )?;
            value.i()
        })();
        result.unwrap_or(-1)
    }

    fn refresh(&mut self, verbose: bool) {
        let fd = self.refresh_fd(verbose);
        let Ok(mut files) = self.files.lock() else {
            return;
        };
        if verbose {
            files.verbose_count = 0;
            let end = format!("-----part {} end----\n", files.verbose_part);
            files.verbose.write_all(end.as_bytes());
            files.verbose.flush();
            files.verbose = Output::from_fd(fd);
            files.verbose_part += 1;
            let start = format!("----part {} start----\n", files.verbose_part);
            files.verbose.write_all(start.as_bytes());
            files.verbose.flush();
        } else {
            files.modules_count = 0;
            let end = format!("-----part {} end----\n", files.modules_part);
            files.modules.write_all(end.as_bytes());
            files.modules.flush();
            files.modules = Output::from_fd(fd);
            files.modules_part += 1;
            let start = format!("----part {} start----\n", files.modules_part);
            files.modules.write_all(start.as_bytes());
            files.modules.flush();
        }
    }

    fn log(&self, data: &[u8]) {
        if let Ok(mut files) = self.files.lock() {
            files.log(data);
        }
    }

    fn print_line(&self, entry: &AndroidLogEntry, verbose: bool) {
        let line = format_line(entry);
        if let Ok(mut files) = self.files.lock() {
            if verbose {
                files.write_verbose(&line);
            } else {
                files.write_modules(&line);
            }
        }
    }

    fn process(&mut self, message: &LogMsg) {
        let entry = unsafe { &*(message.0.as_ptr() as *const LoggerEntry) };
        let mut parsed: AndroidLogEntry = unsafe { std::mem::zeroed() };
        if unsafe { android_log_processLogBuffer(entry, &mut parsed) } < 0 {
            return;
        }
        parsed.tag_len = parsed.tag_len.saturating_sub(1);

        let tag = if parsed.tag.is_null() || parsed.tag_len == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(parsed.tag as *const u8, parsed.tag_len) }
        };
        let message = if parsed.message.is_null() || parsed.message_len == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(parsed.message as *const u8, parsed.message_len) }
        };

        let mut shortcut = false;
        if tag == TAG_BRIDGE || tag == TAG_XSHARED || tag == TAG_CONTEXT {
            self.print_line(&parsed, false);
            shortcut = true;
        }

        let verbose_enabled = self
            .files
            .lock()
            .map(|files| files.verbose_enabled)
            .unwrap_or(true);
        if verbose_enabled
            && (shortcut
                || entry.lid as c_int == LOG_ID_CRASH
                || entry.pid == self.my_pid
                || tag == TAG_MAGISK
                || tag == TAG_DOBBY
                || tag.starts_with(TAG_ZYGISK)
                || tag == TAG_LSPLANT
                || tag.starts_with(TAG_LSPOSED))
        {
            self.print_line(&parsed, true);
        }

        if entry.pid == self.my_pid && tag == TAG_LOGCAT {
            if message == MSG_START_VERBOSE {
                if let Ok(mut files) = self.files.lock() {
                    files.verbose_enabled = true;
                }
                self.print_line(&parsed, true);
            } else if message == MSG_STOP_VERBOSE {
                if let Ok(mut files) = self.files.lock() {
                    files.verbose_enabled = false;
                }
            } else if message == MSG_REFRESH_MODULES {
                self.refresh(false);
            } else if message == MSG_REFRESH_VERBOSE {
                self.refresh(true);
            }
        }
    }

    fn on_crash(&self, error: c_int) {
        static CRASH_COUNT: AtomicUsize = AtomicUsize::new(0);
        static RESTART_WAIT: AtomicUsize = AtomicUsize::new(1 << 3);

        let count = CRASH_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        let wait = RESTART_WAIT.load(Ordering::Relaxed);
        if count >= wait {
            self.log(b"\nLogd crashed too many times, trying manually start...\n");
            set_str_prop(PROP_CTL_RESTART, b"logd");
            if wait < 1 << 10 {
                RESTART_WAIT.store(wait << 1, Ordering::Relaxed);
            } else {
                CRASH_COUNT.store(0, Ordering::Relaxed);
            }
        } else {
            let message = format!(
                "\nLogd maybe crashed (err={}), retrying in 1s...\n",
                io::Error::from_raw_os_error(error)
            );
            self.log(message.as_bytes());
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    fn ensure_watchdog(&self) {
        let files = self.files.clone();
        let _ = std::thread::Builder::new()
            .name("watchdog".into())
            .spawn(move || watchdog(files));
    }

    fn run(mut self) -> ! {
        self.refresh(true);
        self.refresh(false);
        self.ensure_watchdog();

        let mut tail: c_uint = 0;
        loop {
            let list = unsafe { android_logger_list_alloc(0, tail, 0) };
            tail = 10;
            if list.is_null() {
                self.on_crash(libc::ENOMEM);
                continue;
            }

            for id in [LOG_ID_MAIN, LOG_ID_CRASH] {
                let logger = unsafe { android_logger_open(list, id) };
                if logger.is_null() {
                    continue;
                }
                let size = unsafe { android_logger_get_log_size(logger) };
                if size >= 0 && (size as usize) < LOG_BUFFER_SIZE {
                    unsafe { android_logger_set_log_size(logger, LOG_BUFFER_SIZE as libc::c_ulong) };
                }
            }

            let mut message = LogMsg([0; LOGGER_ENTRY_MAX_LEN + 1]);
            loop {
                let read = unsafe {
                    android_logger_list_read(list, &mut message as *mut _ as *mut c_void)
                };
                if read <= 0 {
                    break;
                }
                self.process(&message);

                let (verbose_full, modules_full) = self
                    .files
                    .lock()
                    .map(|files| {
                        (
                            files.verbose_count >= MAX_LOG_SIZE,
                            files.modules_count >= MAX_LOG_SIZE,
                        )
                    })
                    .unwrap_or((false, false));
                if verbose_full {
                    self.refresh(true);
                }
                if modules_full {
                    self.refresh(false);
                }
            }

            unsafe { android_logger_list_free(list) };
            let error = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            self.on_crash(error);
        }
    }
}

fn format_line(entry: &AndroidLogEntry) -> Vec<u8> {
    let mut seconds = entry.tv_sec;
    let mut nanos = entry.tv_nsec;
    if seconds < 0 {
        nanos = NS_PER_SEC - nanos;
    }

    let mut time: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&seconds, &mut time) };
    let mut time_buffer = [0 as c_char; 64];
    unsafe {
        libc::strftime(
            time_buffer.as_mut_ptr(),
            time_buffer.len(),
            b"%Y-%m-%dT%H:%M:%S\0".as_ptr().cast(),
            &time,
        );
    }
    let time = unsafe { CStr::from_ptr(time_buffer.as_ptr()) }.to_string_lossy();

    let tag = if entry.tag.is_null() || entry.tag_len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(entry.tag as *const u8, entry.tag_len) }
    };
    let mut message_len = entry.message_len;
    let message = if entry.message.is_null() || message_len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(entry.message as *const u8, message_len) }
    };
    if message_len >= 1 && message[message_len - 1] == b'\n' {
        message_len -= 1;
    }
    let message = &message[..message_len];

    let level = if (0..LOG_CHARS.len() as c_int).contains(&entry.priority) {
        LOG_CHARS[entry.priority as usize]
    } else {
        b'?'
    };

    let mut line = Vec::with_capacity(128 + message.len());
    let _ = write!(
        &mut line,
        "[ {}.{:03} {:8}:{:6}:{:6} {}/",
        time,
        nanos / MS_PER_NSEC,
        entry.uid,
        entry.pid,
        entry.tid,
        level as char
    );
    line.extend_from_slice(tag);
    if tag.len() < 15 {
        line.resize(line.len() + (15 - tag.len()), b' ');
    }
    line.extend_from_slice(b" ] ");
    line.extend_from_slice(message);
    line.push(b'\n');
    line
}

fn watchdog(files: Arc<Mutex<Files>>) {
    loop {
        let mut need_reset = false;
        if prop_exists(PROP_LOGD_SIZE)
            && get_byte_prop(PROP_LOGD_SIZE).is_some_and(|size| size <= LOG_BUFFER_SIZE)
        {
            set_int_prop(PROP_LOGD_SIZE, LOG_BUFFER_SIZE);
            need_reset = true;
        }
        if prop_exists(PROP_LOGD_MAIN_SIZE)
            && get_byte_prop(PROP_LOGD_MAIN_SIZE).is_some_and(|size| size <= LOG_BUFFER_SIZE)
        {
            set_int_prop(PROP_LOGD_MAIN_SIZE, LOG_BUFFER_SIZE);
            need_reset = true;
        }
        if prop_exists(PROP_LOGD_CRASH_SIZE)
            && get_byte_prop(PROP_LOGD_CRASH_SIZE).is_some_and(|size| size <= LOG_BUFFER_SIZE)
        {
            set_int_prop(PROP_LOGD_CRASH_SIZE, LOG_BUFFER_SIZE);
            need_reset = true;
        }
        if prop_exists(PROP_LOGD_TAG) {
            set_str_prop(PROP_LOGD_TAG, b"");
            need_reset = true;
        }
        if need_reset {
            set_str_prop(PROP_CTL_START, b"logd-reinit");
        }

        let info = unsafe { __system_property_find(PROP_LOGD_TAG.as_ptr().cast()) };
        let mut serial = 0u32;
        if !info.is_null() {
            unsafe {
                __system_property_read_callback(
                    info,
                    read_serial,
                    &mut serial as *mut _ as *mut c_void,
                )
            };
        }

        let mut new_serial = 0u32;
        if !unsafe { __system_property_wait(info, serial, &mut new_serial, ptr::null()) } {
            break;
        }
        if let Ok(mut files) = files.lock() {
            if !info.is_null() {
                files.log(b"\nResetting log settings\n");
            }
        }
        if info.is_null() {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

extern "C" fn read_serial(cookie: *mut c_void, _name: *const c_char, _value: *const c_char, serial: u32) {
    unsafe { *(cookie as *mut u32) = serial };
}

#[no_mangle]
pub extern "system" fn Java_org_lsposed_lspd_service_LogcatService_runLogcat(
    env: JNIEnv,
    thiz: JObject,
) {
    let my_pid = unsafe { libc::getpid() };
    let logcat = Logcat {
        env,
        thiz,
        files: Arc::new(Mutex::new(Files::new())),
        my_pid,
    };
    logcat.run()
}
