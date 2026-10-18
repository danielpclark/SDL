// Rust translation of src/process/windows/SDL_windowsprocess.c (and, for the
// pipes' streams, the Win32 handle backend of src/io/SDL_iostream.c) from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows process backend: `CreateProcessW()` with the standard
//! handles in `STARTUPINFOW`, anonymous pipes (non-blocking on the
//! application's side) for [`ProcessIo::App`], `WaitForSingleObject()` and
//! `TerminateProcess()`.

use std::os::windows::io::IntoRawHandle;

use windows_sys::core::BOOL;

use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, GetLastError, SetHandleInformation, DUPLICATE_SAME_ACCESS,
    ERROR_BROKEN_PIPE, ERROR_NO_DATA, GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT,
    HWND, INVALID_HANDLE_VALUE, LPARAM, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, GetFileType, ReadFile, SetFilePointer, WriteFile, FILE_CURRENT,
    FILE_TYPE_PIPE, INVALID_SET_FILE_POINTER, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    GenerateConsoleCtrlEvent, GetStdHandle, CTRL_BREAK_EVENT, STD_ERROR_HANDLE, STD_HANDLE,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Pipes::{
    CreatePipe, SetNamedPipeHandleState, NAMED_PIPE_MODE, PIPE_NOWAIT, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetCurrentProcess, GetExitCodeProcess, TerminateProcess, WaitForSingleObject,
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, INFINITE,
    PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, PostMessageW, PostThreadMessageW, WM_CLOSE,
};

use super::{Process, ProcessBuilder, ProcessIo, PROP_PROCESS_PID_NUMBER};
use crate::core::windows::{set_error, utf8_to_wide};
use crate::error::{Error, Result};
use crate::io::{IoInterface, IoStatus, IoStop, IoStream};
use crate::stdlib::Environment;

const READ_END: usize = 0;
const WRITE_END: usize = 1;

/// Translation of `struct SDL_ProcessData`; the handles close on drop (the
/// end of `SDL_SYS_DestroyProcess()`).
pub(super) struct ProcessData {
    process_information: PROCESS_INFORMATION,
}

// SAFETY: process and thread handles are process-wide kernel object handles,
// usable from any thread.
unsafe impl Send for ProcessData {}
// SAFETY: as above; nothing is mutated through a shared reference.
unsafe impl Sync for ProcessData {}

impl std::fmt::Debug for ProcessData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessData")
            .field("pid", &self.process_information.dwProcessId)
            .finish_non_exhaustive()
    }
}

impl Drop for ProcessData {
    fn drop(&mut self) {
        let info = &self.process_information;
        // SAFETY: the handles came from CreateProcessW() and are closed once.
        unsafe {
            if info.hThread != INVALID_HANDLE_VALUE {
                CloseHandle(info.hThread);
            }
            if info.hProcess != INVALID_HANDLE_VALUE {
                CloseHandle(info.hProcess);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The streams of the application's ends of the pipes: `SDL_IOFromHandle()`
// on a pipe handle (no size or seek, which are only set for disk files).
// ---------------------------------------------------------------------------

/// Translation of `READAHEAD_BUFFER_SIZE`.
const READAHEAD_BUFFER_SIZE: usize = 1024;
/// Translation of `WRITEBEHIND_BUFFER_SIZE`.
const WRITEBEHIND_BUFFER_SIZE: usize = 1024;

/// Translation of `IOStreamWindowsData`.
struct HandleStream {
    h: HANDLE,
    data: Box<[u8; READAHEAD_BUFFER_SIZE]>,
    size: usize,
    left: usize,
    write_data: Box<[u8; WRITEBEHIND_BUFFER_SIZE]>,
    write_pos: usize,
    writable: bool,
    append: bool,
    autoclose: bool,
}

// SAFETY: the handle is a process-wide kernel object handle, used by one
// stream (behind its `&mut self`).
unsafe impl Send for HandleStream {}

impl HandleStream {
    /// Translation of `SDL_IOFromHandle()`.
    fn open(handle: HANDLE, mode: &str, autoclose: bool) -> IoStream<'static> {
        IoStream::open(HandleStream::new(handle, mode, autoclose))
    }

    /// The stream's data (`SDL_IOFromHandle()` before the stream is made).
    fn new(handle: HANDLE, mode: &str, autoclose: bool) -> HandleStream {
        HandleStream {
            h: handle,
            data: Box::new([0; READAHEAD_BUFFER_SIZE]),
            size: 0,
            left: 0,
            write_data: Box::new([0; WRITEBEHIND_BUFFER_SIZE]),
            write_pos: 0,
            writable: mode.contains('w') || mode.contains('a') || mode.contains('+'),
            append: mode.contains('a'),
            autoclose,
        }
    }

    /// Translation of `windows_flush_write_buffer()`.
    fn flush_write_buffer(&mut self) -> std::result::Result<(), IoStop> {
        if self.write_pos == 0 {
            return Ok(()); // Nothing to flush
        }

        // In append mode, seek to EOF before writing
        // (a pipe has no end to seek to: the stream is never opened in
        // append mode here)
        debug_assert!(!self.append);

        let mut bytes = 0u32;
        // SAFETY: write_data holds write_pos initialized bytes; no OVERLAPPED.
        let ok = unsafe {
            WriteFile(
                self.h,
                self.write_data.as_ptr(),
                self.write_pos as u32,
                &mut bytes,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(IoStop::error(0, set_error("Error writing to datastream")));
        }

        // Note (upstream): a short write (to a non-blocking pipe that is
        // full) drops the bytes that weren't written; the translation keeps
        // them buffered and reports the stream not ready.
        let bytes = bytes as usize;
        self.write_data.copy_within(bytes..self.write_pos, 0);
        self.write_pos -= bytes;
        if self.write_pos != 0 {
            return Err(IoStop::status(0, IoStatus::NotReady));
        }
        Ok(())
    }

    /// The `ReadFile()` failure of `windows_file_read()`.
    fn read_error() -> IoStop {
        // SAFETY: GetLastError has no preconditions.
        match unsafe { GetLastError() } {
            ERROR_BROKEN_PIPE => IoStop::status(0, IoStatus::Eof),
            ERROR_NO_DATA => IoStop::status(0, IoStatus::NotReady),
            _ => IoStop::error(0, set_error("Error reading from datastream")),
        }
    }
}

impl IoInterface for HandleStream {
    fn can_read(&self) -> bool {
        true
    }

    /// Translation of `windows_file_read()`.
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let size = buf.len();
        let mut total_need = size;
        let mut total_read = 0usize;
        let mut ptr = 0usize;
        let mut bytes = 0u32;

        self.flush_write_buffer()?;

        if self.left > 0 {
            let start = self.size - self.left;
            let read_ahead = total_need.min(self.left);
            buf[..read_ahead].copy_from_slice(&self.data[start..start + read_ahead]);
            self.left -= read_ahead;

            if read_ahead == total_need {
                return Ok(size);
            }
            ptr += read_ahead;
            total_need -= read_ahead;
            total_read += read_ahead;
        }

        let eof;
        if total_need < READAHEAD_BUFFER_SIZE {
            // SAFETY: data holds READAHEAD_BUFFER_SIZE bytes; no OVERLAPPED.
            let ok = unsafe {
                ReadFile(
                    self.h,
                    self.data.as_mut_ptr(),
                    READAHEAD_BUFFER_SIZE as u32,
                    &mut bytes,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                // !!! FIXME: this should return the bytes read from any readahead we finished out before this (the `iodata->left > 0` code above). In that case, fail on the next read.
                return Err(Self::read_error());
            }
            eof = bytes == 0;
            let bytes = bytes as usize;
            let read_ahead = total_need.min(bytes);
            buf[ptr..ptr + read_ahead].copy_from_slice(&self.data[..read_ahead]);
            self.size = bytes;
            self.left = bytes - read_ahead;
            total_read += read_ahead;
        } else {
            // SAFETY: buf[ptr..] holds total_need bytes; no OVERLAPPED.
            let ok = unsafe {
                ReadFile(
                    self.h,
                    buf[ptr..].as_mut_ptr(),
                    total_need as u32,
                    &mut bytes,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                // !!! FIXME: this should return the bytes read from any readahead we finished out before this (the `iodata->left > 0` code above). In that case, fail on the next read.
                return Err(Self::read_error());
            }
            eof = bytes == 0;
            total_read += bytes as usize;
        }
        if eof {
            Err(IoStop::status(total_read, IoStatus::Eof))
        } else {
            Ok(total_read)
        }
    }

    fn can_write(&self) -> bool {
        true
    }

    /// Translation of `windows_file_write()`.
    fn write(&mut self, src: &[u8]) -> std::result::Result<usize, IoStop> {
        let size = src.len();
        let mut remaining = size;
        let mut total_written = 0usize;
        let mut p = 0usize;

        if !self.writable {
            return Err(IoStop::status(0, IoStatus::ReadOnly));
        }

        // Invalidate read-ahead buffer if it has data
        if self.left != 0 {
            // SAFETY: SetFilePointer takes the handle and plain values.
            let ret = unsafe {
                SetFilePointer(
                    self.h,
                    -(self.left as i32),
                    std::ptr::null_mut(),
                    FILE_CURRENT,
                )
            };
            // Note (upstream): `!SetFilePointer()` is the check there, but
            // it fails with INVALID_SET_FILE_POINTER, and 0 is the valid
            // offset of a write after a read-ahead from the start (of a
            // file: the process's pipes are read or written, not both);
            // the translation checks for the failure only.
            if ret == INVALID_SET_FILE_POINTER {
                return Err(IoStop::error(0, set_error("Error seeking in datastream")));
            }
            self.left = 0;
        }

        // For large writes, flush buffer and write directly
        if size >= WRITEBEHIND_BUFFER_SIZE {
            self.flush_write_buffer()?;

            // In append mode, seek to EOF before direct write
            // (see flush_write_buffer())

            let mut bytes = 0u32;
            // SAFETY: src holds size bytes; no OVERLAPPED.
            let ok = unsafe {
                WriteFile(
                    self.h,
                    src.as_ptr(),
                    size as u32,
                    &mut bytes,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(IoStop::error(0, set_error("Error writing to datastream")));
            } else if bytes == 0 && size > 0 {
                return Err(IoStop::status(0, IoStatus::NotReady));
            }
            return Ok(bytes as usize);
        }

        // Buffer small writes
        while remaining > 0 {
            let space = WRITEBEHIND_BUFFER_SIZE - self.write_pos;
            let to_buffer = remaining.min(space);

            self.write_data[self.write_pos..self.write_pos + to_buffer]
                .copy_from_slice(&src[p..p + to_buffer]);
            self.write_pos += to_buffer;
            p += to_buffer;
            remaining -= to_buffer;
            total_written += to_buffer;

            if self.write_pos == WRITEBEHIND_BUFFER_SIZE {
                match self.flush_write_buffer() {
                    Ok(()) => {}
                    // (written in part: the rest stays buffered)
                    Err(stop)
                        if stop.status == IoStatus::NotReady
                            && self.write_pos < WRITEBEHIND_BUFFER_SIZE => {}
                    Err(mut stop) => {
                        stop.bytes = total_written;
                        return Err(stop);
                    }
                }
            }
        }

        Ok(total_written)
    }

    /// Translation of `windows_file_flush()`.
    fn flush(&mut self) -> std::result::Result<(), IoStop> {
        self.flush_write_buffer()?;

        // Sync to disk
        // (for a pipe, this waits for the other end to read everything)
        // SAFETY: FlushFileBuffers takes the handle.
        if unsafe { FlushFileBuffers(self.h) } == 0 {
            return Err(IoStop::error(0, set_error("Error flushing datastream")));
        }
        Ok(())
    }

    /// Translation of `windows_file_close()`.
    fn close(&mut self) -> Result<()> {
        let mut result = Ok(());

        // (what a full pipe didn't take is written in blocking mode, so the
        // bytes buffered and reported written aren't dropped)
        if self.write_pos != 0 {
            let wait_mode: NAMED_PIPE_MODE = PIPE_WAIT;
            // SAFETY: the stream's handle; the mode outlives the call. (A
            // handle that isn't a pipe's is left as it is: its writes block.)
            unsafe {
                SetNamedPipeHandleState(self.h, &wait_mode, std::ptr::null(), std::ptr::null())
            };
        }
        if let Err(stop) = self.flush_write_buffer() {
            result = Err(stop
                .error
                .unwrap_or_else(|| Error::new("Error writing to datastream")));
        }

        if self.h != INVALID_HANDLE_VALUE {
            if self.autoclose {
                // SAFETY: the stream owns the handle; it is closed once.
                unsafe { CloseHandle(self.h) };
            }
            self.h = INVALID_HANDLE_VALUE;
        }
        result
    }
}

/// Translation of `SetupStream()` (the stream is the process's, so there is
/// no property to clear when it is closed).
fn setup_stream(handle: HANDLE, mode: &str) -> IoStream<'static> {
    HandleStream::open(handle, mode, true)
}

/// Translation of `SetupRedirect()`: an inheritable duplicate of the file's
/// handle (which closes with `file`), in blocking mode if it is a pipe.
fn setup_redirect(file: std::fs::File, result: &mut HANDLE) -> Result<()> {
    let handle = file.into_raw_handle() as HANDLE;
    // (the application's handle is closed once duplicated: the File was
    // given to the builder)
    let _owned = OwnedHandle(handle);

    // SAFETY: handle is a live handle; result receives the duplicate.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            result,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(set_error("DuplicateHandle()"));
    }

    // SAFETY: *result is the live duplicate.
    if unsafe { GetFileType(*result) } == FILE_TYPE_PIPE {
        let wait_mode: NAMED_PIPE_MODE = PIPE_WAIT;
        // SAFETY: as above; the mode outlives the call.
        if unsafe {
            SetNamedPipeHandleState(*result, &wait_mode, std::ptr::null(), std::ptr::null())
        } == 0
        {
            return Err(set_error("SetNamedPipeHandleState()"));
        }
    }
    Ok(())
}

/// A handle closed on drop.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if self.0 != INVALID_HANDLE_VALUE && !self.0.is_null() {
            // SAFETY: the handle is owned here and closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Translation of `is_batch_file_path()`.
fn is_batch_file_path(path: &str) -> bool {
    let len_path = path.len();
    if len_path < 4 || !path.is_char_boundary(len_path - 4) {
        return false;
    }
    let ext = &path[len_path - 4..];
    ext.eq_ignore_ascii_case(".bat") || ext.eq_ignore_ascii_case(".cmd")
}

/// The command line of `args`, quoted and escaped for
/// `CommandLineToArgvW()` (and for `cmd.exe`, if `args[0]` is a batch
/// file). Translation of `join_arguments()`, without the conversion to
/// UTF-16.
fn join_arguments_utf8(args: &[String]) -> String {
    let batch_file = is_batch_file_path(&args[0]);

    // (upstream counts the length first, then fills the buffer)
    let mut result = String::new();
    for a in args {
        let quotes = a.is_empty() || a.contains([' ', '\r', '\n', '\t', '\x0b']);

        if quotes {
            /* surround the argument with double quote if it is empty or contains whitespaces */
            result.push('"');
        }
        let bytes = a.as_bytes();
        for (i, c) in a.char_indices() {
            let next = bytes.get(i + c.len_utf8()).copied();
            match c {
                '"' => {
                    if batch_file {
                        result.push('"');
                    } else {
                        result.push('\\');
                    }
                    result.push(c);
                }
                '\\' => {
                    result.push(c);
                    /* only escape backslashes that precede a double quote (including the enclosing double quote) */
                    if next == Some(b'"') || (quotes && next.is_none()) {
                        result.push(c);
                    }
                }
                ' ' => {
                    if batch_file {
                        result.push('^');
                    }
                    result.push(c);
                }
                '^' | '&' | '|' | '<' | '>' => {
                    if batch_file {
                        result.push('^');
                    }
                    result.push(c);
                }
                _ => result.push(c),
            }
        }
        if quotes {
            result.push('"');
        }
        result.push(' ');
    }
    result.pop();
    result
}

/// Translation of `join_arguments()`.
fn join_arguments(args: &[String]) -> Vec<u16> {
    // Note (upstream): the conversion's failure is checked as `!args_out`
    // (the out pointer) rather than `!*args_out`; a conversion here can't fail.
    utf8_to_wide(&join_arguments_utf8(args))
}

/// The environment block for `CREATE_UNICODE_ENVIRONMENT`: each
/// `NAME=value` with its NUL, and a final NUL. Translation of `join_env()`.
fn join_env(env: &[String]) -> Vec<u16> {
    let mut result = Vec::new();
    for var in env {
        result.extend(var.encode_utf16());
        result.push(0);
    }
    // (SDL_iconv_string() terminates its UTF-16 result with two more NULs,
    // which makes an empty block "\0\0" as CreateProcessW() needs)
    result.extend([0, 0]);
    result
}

/// `CreateFile(TEXT("\\\\.\\NUL"), ...)`, inheritable.
fn open_nul(security_attributes: &SECURITY_ATTRIBUTES) -> HANDLE {
    let name = utf8_to_wide("\\\\.\\NUL");
    // SAFETY: name is NUL-terminated; the attributes outlive the call.
    unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            security_attributes,
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    }
}

/// An inheritable duplicate of the application's standard handle
/// (`NULL` if it has none). The `SDL_PROCESS_STDIO_INHERITED` case.
fn duplicate_std_handle(which: STD_HANDLE, result: &mut HANDLE) -> Result<()> {
    // SAFETY: GetStdHandle takes a constant.
    let handle = unsafe { GetStdHandle(which) };
    duplicate_inheritable(handle, result)
}

/// `DuplicateHandle()` into an inheritable handle, or `NULL` for `NULL`.
fn duplicate_inheritable(handle: HANDLE, result: &mut HANDLE) -> Result<()> {
    if handle.is_null() {
        *result = std::ptr::null_mut();
    } else if {
        // SAFETY: handle is the caller's; result receives the duplicate.
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                handle,
                GetCurrentProcess(),
                result,
                0,
                1,
                DUPLICATE_SAME_ACCESS,
            )
        }
    } == 0
    {
        *result = INVALID_HANDLE_VALUE;
        return Err(set_error("DuplicateHandle()"));
    }
    Ok(())
}

/// The `SDL_PROCESS_STDIO_APP` case: a pipe whose application end
/// (`app_end`) is non-blocking and not inherited, and whose other end goes
/// to the child.
fn create_app_pipe(
    pipe: &mut [HANDLE; 2],
    app_end: usize,
    security_attributes: &SECURITY_ATTRIBUTES,
) -> Result<HANDLE> {
    let pipe_mode: NAMED_PIPE_MODE = PIPE_NOWAIT;
    let (mut read, mut write) = (INVALID_HANDLE_VALUE, INVALID_HANDLE_VALUE);
    // SAFETY: the out pointers and attributes are valid for the call.
    if unsafe { CreatePipe(&mut read, &mut write, security_attributes, 0) } == 0 {
        pipe[READ_END] = INVALID_HANDLE_VALUE;
        pipe[WRITE_END] = INVALID_HANDLE_VALUE;
        // (upstream returns without setting an error)
        return Err(set_error("CreatePipe()"));
    }
    pipe[READ_END] = read;
    pipe[WRITE_END] = write;
    // SAFETY: the pipe's handles are live; the mode outlives the call.
    unsafe {
        if SetNamedPipeHandleState(
            pipe[app_end],
            &pipe_mode,
            std::ptr::null(),
            std::ptr::null(),
        ) == 0
        {
            return Err(set_error("SetNamedPipeHandleState()"));
        }
        if SetHandleInformation(pipe[app_end], HANDLE_FLAG_INHERIT, 0) == 0 {
            return Err(set_error("SetHandleInformation()"));
        }
    }
    Ok(pipe[1 - app_end])
}

/// Translation of `SDL_SYS_CreateProcessWithProperties()`.
pub(super) fn sys_create_process(process: &mut Process, builder: ProcessBuilder) -> Result<()> {
    let ProcessBuilder {
        args,
        environment,
        working_directory,
        stdin: mut stdin_option,
        stdout: mut stdout_option,
        stderr,
        stderr_to_stdout,
        background,
        cmdline,
    } = builder;
    let env = environment.unwrap_or_else(Environment::process);
    let redirect_stderr = stderr_to_stdout && stderr.is_none();
    let mut stderr_option = stderr.unwrap_or_default();
    let mut stdin_pipe = [INVALID_HANDLE_VALUE; 2];
    let mut stdout_pipe = [INVALID_HANDLE_VALUE; 2];
    let mut stderr_pipe = [INVALID_HANDLE_VALUE; 2];

    // Keep the malloc() before exec() so that an OOM won't run a process at all
    let envp = env.variables();

    process.internal = Some(ProcessData {
        process_information: PROCESS_INFORMATION {
            hProcess: INVALID_HANDLE_VALUE,
            hThread: INVALID_HANDLE_VALUE,
            dwProcessId: 0,
            dwThreadId: 0,
        },
    });

    // (and a process group of its own, for the break of sys_kill_process();
    // as the first process of a group, it ignores CTRL+C until it asks for it)
    let mut creation_flags = CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP;

    // SAFETY: STARTUPINFOW is plain data; all zeroes is valid.
    let mut startup_info: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup_info.cb = size_of::<STARTUPINFOW>() as u32;
    startup_info.dwFlags |= STARTF_USESTDHANDLES;
    startup_info.hStdInput = INVALID_HANDLE_VALUE;
    startup_info.hStdOutput = INVALID_HANDLE_VALUE;
    startup_info.hStdError = INVALID_HANDLE_VALUE;

    let security_attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };

    // (the body up to `done:`)
    let result = (|| -> Result<()> {
        let mut createprocess_cmdline = match &cmdline {
            Some(cmdline) => utf8_to_wide(cmdline),
            None => join_arguments(&args),
        };

        let createprocess_env = join_env(&envp);

        let createprocess_cwd = working_directory.as_deref().map(utf8_to_wide);

        // Background processes don't have access to the terminal
        // This isn't necessary on Windows, but we keep the same behavior as the POSIX implementation.
        if background {
            if stdin_option.is_inherited() {
                stdin_option = ProcessIo::Null;
            }
            if stdout_option.is_inherited() {
                stdout_option = ProcessIo::Null;
            }
            if stderr_option.is_inherited() {
                stderr_option = ProcessIo::Null;
            }
            creation_flags |= CREATE_NO_WINDOW;
        }

        let stdin_app = matches!(stdin_option, ProcessIo::App);
        let stdout_app = matches!(stdout_option, ProcessIo::App);
        let stderr_app = !redirect_stderr && matches!(stderr_option, ProcessIo::App);

        match stdin_option {
            ProcessIo::Redirect(file) => {
                setup_redirect(file, &mut startup_info.hStdInput)?;
            }
            ProcessIo::App => {
                startup_info.hStdInput =
                    create_app_pipe(&mut stdin_pipe, WRITE_END, &security_attributes)?;
            }
            ProcessIo::Null => {
                startup_info.hStdInput = open_nul(&security_attributes);
            }
            ProcessIo::Inherited => {
                duplicate_std_handle(STD_INPUT_HANDLE, &mut startup_info.hStdInput)?;
            }
        }

        match stdout_option {
            ProcessIo::Redirect(file) => {
                setup_redirect(file, &mut startup_info.hStdOutput)?;
            }
            ProcessIo::App => {
                startup_info.hStdOutput =
                    create_app_pipe(&mut stdout_pipe, READ_END, &security_attributes)?;
            }
            ProcessIo::Null => {
                startup_info.hStdOutput = open_nul(&security_attributes);
            }
            ProcessIo::Inherited => {
                duplicate_std_handle(STD_OUTPUT_HANDLE, &mut startup_info.hStdOutput)?;
            }
        }

        if redirect_stderr {
            duplicate_inheritable(startup_info.hStdOutput, &mut startup_info.hStdError)?;
        } else {
            match stderr_option {
                ProcessIo::Redirect(file) => {
                    setup_redirect(file, &mut startup_info.hStdError)?;
                }
                ProcessIo::App => {
                    startup_info.hStdError =
                        create_app_pipe(&mut stderr_pipe, READ_END, &security_attributes)?;
                }
                ProcessIo::Null => {
                    startup_info.hStdError = open_nul(&security_attributes);
                }
                ProcessIo::Inherited => {
                    duplicate_std_handle(STD_ERROR_HANDLE, &mut startup_info.hStdError)?;
                }
            }
        }

        let data = process.internal.as_mut().expect("set above");
        // SAFETY: the command line is a mutable NUL-terminated buffer, the
        // environment block is double-NUL-terminated UTF-16, the directory
        // is NUL-terminated, and the structures outlive the call.
        if unsafe {
            CreateProcessW(
                std::ptr::null(),
                createprocess_cmdline.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                creation_flags,
                createprocess_env.as_ptr().cast(),
                createprocess_cwd
                    .as_ref()
                    .map_or(std::ptr::null(), |cwd| cwd.as_ptr()),
                &startup_info,
                &mut data.process_information,
            )
        } == 0
        {
            return Err(set_error("CreateProcess"));
        }

        process.props.set(
            PROP_PROCESS_PID_NUMBER,
            i64::from(data.process_information.dwProcessId),
        )?;

        if stdin_app {
            process.stdin = Some(setup_stream(stdin_pipe[WRITE_END], "wb"));
            stdin_pipe[WRITE_END] = INVALID_HANDLE_VALUE;
        }
        if stdout_app {
            process.stdout = Some(setup_stream(stdout_pipe[READ_END], "rb"));
            stdout_pipe[READ_END] = INVALID_HANDLE_VALUE;
        }
        if stderr_app {
            process.stderr = Some(setup_stream(stderr_pipe[READ_END], "rb"));
            stderr_pipe[READ_END] = INVALID_HANDLE_VALUE;
        }

        Ok(())
    })();

    // done:
    // SAFETY: each handle closed here is owned by this function and closed
    // once (the application's ends given to streams were reset above).
    unsafe {
        if startup_info.hStdInput != INVALID_HANDLE_VALUE
            && startup_info.hStdInput != stdin_pipe[READ_END]
        {
            CloseHandle(startup_info.hStdInput);
        }
        if startup_info.hStdOutput != INVALID_HANDLE_VALUE
            && startup_info.hStdOutput != stdout_pipe[WRITE_END]
        {
            CloseHandle(startup_info.hStdOutput);
        }
        if startup_info.hStdError != INVALID_HANDLE_VALUE
            && startup_info.hStdError != stderr_pipe[WRITE_END]
        {
            CloseHandle(startup_info.hStdError);
        }
        if stdin_pipe[READ_END] != INVALID_HANDLE_VALUE {
            CloseHandle(stdin_pipe[READ_END]);
        }
        if stdout_pipe[WRITE_END] != INVALID_HANDLE_VALUE {
            CloseHandle(stdout_pipe[WRITE_END]);
        }
        if stderr_pipe[WRITE_END] != INVALID_HANDLE_VALUE {
            CloseHandle(stderr_pipe[WRITE_END]);
        }

        if result.is_err() {
            if stdin_pipe[WRITE_END] != INVALID_HANDLE_VALUE {
                CloseHandle(stdin_pipe[WRITE_END]);
            }
            if stdout_pipe[READ_END] != INVALID_HANDLE_VALUE {
                CloseHandle(stdout_pipe[READ_END]);
            }
            if stderr_pipe[READ_END] != INVALID_HANDLE_VALUE {
                CloseHandle(stderr_pipe[READ_END]);
            }
        }
    }
    result
}

/// Translation of `terminate_app()`.
unsafe extern "system" fn terminate_app(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the [u32; 2] passed to EnumWindows().
    let term_info = unsafe { &mut *(lparam as *mut [u32; 2]) };
    let mut current_proc_id = 0u32;
    // SAFETY: hwnd is a window being enumerated.
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut current_proc_id);
        if current_proc_id == term_info[0] && PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0 {
            term_info[1] += 1;
        }
    }
    1
}

/// Translation of `SDL_SYS_KillProcess()`.
pub(super) fn sys_kill_process(process: &mut Process, force: bool) -> Result<()> {
    let Some(data) = &process.internal else {
        return Err(Error::new("Process isn't running"));
    };
    let info = &data.process_information;
    if !force {
        // term_info[0] is the process ID, term_info[1] is number of successful tries
        let mut term_info = [info.dwProcessId, 0u32];
        // SAFETY: terminate_app() gets a pointer to term_info, which
        // outlives the enumeration.
        unsafe {
            EnumWindows(
                Some(terminate_app),
                &mut term_info as *mut [u32; 2] as LPARAM,
            );
            if term_info[1] != 0 || PostThreadMessageW(info.dwThreadId, WM_CLOSE, 0, 0) != 0 {
                return Ok(());
            }
            // Note (upstream): the process isn't created with
            // CREATE_NEW_PROCESS_GROUP there, so its ID names no process
            // group, and the break goes to every process on the console
            // (the caller included, which it ends); the translation creates
            // it in a group of its own.
            if GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, term_info[0]) != 0 {
                return Ok(());
            }
        }
    }
    // SAFETY: hProcess is the live process handle.
    if unsafe { TerminateProcess(info.hProcess, 1) } == 0 {
        return Err(set_error("TerminateProcess failed"));
    }
    Ok(())
}

/// Translation of `SDL_SYS_WaitProcess()`.
pub(super) fn sys_wait_process(process: &mut Process, block: bool) -> Result<Option<i32>> {
    let Some(data) = &process.internal else {
        return Ok(Some(-255));
    };
    let h_process = data.process_information.hProcess;

    // SAFETY: hProcess is the live process handle.
    let result = unsafe { WaitForSingleObject(h_process, if block { INFINITE } else { 0 }) };

    if result == WAIT_OBJECT_0 {
        let mut rc = 0u32;
        // SAFETY: as above; rc receives the exit code.
        if unsafe { GetExitCodeProcess(h_process, &mut rc) } == 0 {
            return Err(set_error("GetExitCodeProcess"));
        }
        Ok(Some(rc as i32))
    } else if result == WAIT_FAILED {
        Err(set_error(
            "WaitForSingleObject(hProcess) returned WAIT_FAILED",
        ))
    } else {
        Ok(None)
    }
}

/// Translation of `SDL_SYS_DestroyProcess()`.
pub(super) fn sys_destroy_process(process: &mut Process) {
    if let Some(io) = process.stdin.take() {
        let _ = io.close();
    }
    if let Some(io) = process.stderr.take() {
        let _ = io.close();
    }
    if let Some(io) = process.stdout.take() {
        let _ = io.close();
    }
    process.internal = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::*;
    use std::io::{Read as _, Write as _};

    /// The variable that makes [`child_helper`] act as the child process
    /// (its value says what to do).
    const CHILD_VAR: &str = "SDL3_PROCESS_TEST_CHILD";
    /// What the helper prints before its output (after libtest's own).
    const MARK: &[u8] = b"[[sdl3-child]]";

    /// The pipes' child ends are inheritable while a process is created, so
    /// a process created at the same time by another test could hold them
    /// open (and delay the end of the output): the tests take turns.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// This test binary, re-run as the child process `mode`.
    fn helper(mode: &str) -> ProcessBuilder {
        let env = Environment::new(true);
        env.set(CHILD_VAR, mode, true).unwrap();
        helper_with(env)
    }

    fn helper_with(env: Environment) -> ProcessBuilder {
        let exe = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        ProcessBuilder::new([
            exe.as_str(),
            "process::windows::tests::child_helper",
            "--exact",
            "--quiet",
            "--test-threads=1",
        ])
        .environment(env)
    }

    /// The helper's output, after libtest's.
    fn payload(out: &[u8]) -> &[u8] {
        let at = out
            .windows(MARK.len())
            .position(|w| w == MARK)
            .unwrap_or_else(|| panic!("no helper output in {:?}", String::from_utf8_lossy(out)));
        &out[at + MARK.len()..]
    }

    /// The child process of the other tests (does nothing in a normal run).
    #[test]
    fn child_helper() {
        let Ok(mode) = std::env::var(CHILD_VAR) else {
            return;
        };
        let mut out = std::io::stdout();
        out.write_all(MARK).unwrap();
        match mode.as_str() {
            "exit" => {
                out.write_all(b"hello\n").unwrap();
                out.flush().unwrap();
                std::process::exit(3);
            }
            "echo" => {
                let mut data = Vec::new();
                std::io::stdin().read_to_end(&mut data).unwrap();
                out.write_all(b"got ").unwrap();
                out.write_all(&data).unwrap();
                out.flush().unwrap();
                std::io::stderr().write_all(b"oops\n").unwrap();
                std::process::exit(0);
            }
            "env" => {
                let value = std::env::var("SDL_TEST_VALUE").unwrap_or_default();
                let cwd = std::env::current_dir().unwrap();
                writeln!(out, "{value}\n{}", cwd.display()).unwrap();
                out.flush().unwrap();
                std::process::exit(0);
            }
            "sleep" => {
                out.flush().unwrap();
                std::thread::sleep(std::time::Duration::from_secs(60));
                std::process::exit(0);
            }
            "window" => {
                // A hidden top-level window that ends the process with 42
                // when it is sent WM_CLOSE (what kill(false) posts).
                std::process::exit(close_window_helper(&mut out));
            }
            _ => std::process::exit(99),
        }
    }

    fn close_window_helper(out: &mut std::io::Stdout) -> i32 {
        use windows_sys::Win32::Foundation::{LRESULT, WPARAM};
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
            RegisterClassW, MSG, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        };

        unsafe extern "system" fn wndproc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if msg == WM_CLOSE {
                // SAFETY: called on the window's thread.
                unsafe { PostQuitMessage(42) };
                return 0;
            }
            // SAFETY: the default handling of the window's message.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        let class_name = utf8_to_wide("SDL3ProcessTestWindow");
        // SAFETY: the class name outlives the calls; the window procedure
        // is valid; the message loop runs on the window's thread.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                100,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                let _ = out.write_all(b"nowindow\n");
                let _ = out.flush();
                return 98;
            }
            let _ = out.write_all(b"ready\n");
            let _ = out.flush();
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                DispatchMessageW(&msg);
            }
            msg.wParam as i32
        }
    }

    /// Read the output without blocking until it ends with one of `ends`.
    fn read_until(p: &mut Process, ends: &[&[u8]]) -> Vec<u8> {
        let mut data = Vec::new();
        let start = std::time::Instant::now();
        while !ends.iter().any(|e| data.ends_with(e)) {
            let mut buf = [0u8; 64];
            let out = p.output().unwrap();
            let n = out.read(&mut buf);
            data.extend(&buf[..n]);
            if n == 0 {
                assert_eq!(out.status(), IoStatus::NotReady, "{data:?}");
                assert!(start.elapsed().as_secs() < 60, "no output: {data:?}");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        data
    }

    #[test]
    fn join_arguments_quotes_and_escapes() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            join_arguments_utf8(&args(&["prog", "a", "b c", ""])),
            r#"prog a "b c" """#
        );
        assert_eq!(
            join_arguments_utf8(&args(&[
                "prog",
                r#"say "hi""#,
                r"C:\dir\",
                r"C:\a b\",
                r#"x\"y"#
            ])),
            r#"prog "say \"hi\"" C:\dir\ "C:\a b\\" x\\\"y"#
        );
        assert_eq!(
            join_arguments_utf8(&args(&["prog", "a&b|c<d>e^f", "\tx"])),
            "prog a&b|c<d>e^f \"\tx\""
        );
        // Batch files are run through cmd.exe, which needs ^ escapes.
        assert!(is_batch_file_path("run.BAT") && is_batch_file_path("x.cmd"));
        assert!(!is_batch_file_path("bat") && !is_batch_file_path("run.exe"));
        assert_eq!(
            join_arguments_utf8(&args(&["run.bat", "a b", r#"q"q"#, "x&y"])),
            r#"run.bat "a^ b" q""q x^&y"#
        );
        assert_eq!(join_arguments(&args(&["é"])), ['é' as u16, 0]);
    }

    #[test]
    fn join_env_block() {
        let block = join_env(&["A=1".to_string(), "BC=é".to_string()]);
        let expected: Vec<u16> = "A=1\0BC=é\0\0\0".encode_utf16().collect();
        assert_eq!(block, expected);
        assert_eq!(join_env(&[]), [0, 0]);
    }

    #[test]
    fn read_output_and_exit_code() {
        let _lock = lock();
        let mut p = helper("exit")
            .stdin(ProcessIo::App)
            .stdout(ProcessIo::App)
            .spawn()
            .unwrap();
        assert!(p.properties().get_number(PROP_PROCESS_PID_NUMBER).unwrap() > 0);
        assert!(!p
            .properties()
            .get_bool(PROP_PROCESS_BACKGROUND_BOOLEAN)
            .unwrap());
        let (data, code) = p.read().unwrap();
        assert_eq!(payload(&data), b"hello\n");
        assert_eq!(code, 3);
        assert_eq!(p.wait(false).unwrap(), Some(3));
        assert_eq!(
            p.kill(true).unwrap_err().to_string(),
            "Process isn't running"
        );
    }

    #[test]
    fn cmd_exit_code() {
        let _lock = lock();
        let mut p = Process::create(&["cmd.exe", "/c", "exit 7"], false).unwrap();
        assert_eq!(p.wait(true).unwrap(), Some(7));

        // The command line as is, in place of the arguments.
        let mut p = ProcessBuilder::new(Vec::<String>::new())
            .cmdline("cmd.exe /c exit 9")
            .spawn()
            .unwrap();
        assert_eq!(p.wait(true).unwrap(), Some(9));
    }

    #[test]
    fn input_and_stderr() {
        let _lock = lock();
        let mut p = helper("echo")
            .stdin(ProcessIo::App)
            .stdout(ProcessIo::App)
            .stderr_to_stdout(true)
            .spawn()
            .unwrap();
        assert!(p.error().is_err());
        assert_eq!(p.input().unwrap().write(b"abc\n"), 4);
        p.close_input().unwrap();
        let (data, code) = p.read().unwrap();
        assert_eq!(payload(&data), b"got abc\noops\n");
        assert_eq!(code, 0);

        // Standard error on its own stream.
        let mut p = helper("echo")
            .stdin(ProcessIo::App)
            .stdout(ProcessIo::App)
            .stderr(ProcessIo::App)
            .spawn()
            .unwrap();
        // (bigger than the write-behind buffer: written directly)
        let big = vec![b'x'; 3000];
        assert_eq!(p.input().unwrap().write(&big), big.len());
        p.close_input().unwrap();
        let (data, code) = p.read().unwrap();
        let mut expected = b"got ".to_vec();
        expected.extend(&big);
        assert_eq!(payload(&data), expected);
        assert_eq!(code, 0);
        assert_eq!(p.error().unwrap().load_all().unwrap(), b"oops\n");
    }

    /// Small writes to a child that isn't reading yet fill the pipe: what
    /// it doesn't take stays buffered (upstream drops it), the stream is
    /// not ready once its buffer is full too, and closing it writes the
    /// rest.
    #[test]
    fn input_waits_for_a_full_pipe() {
        let _lock = lock();
        let mut p = helper("echo")
            .stdin(ProcessIo::App)
            .stdout(ProcessIo::App)
            .stderr(ProcessIo::Null)
            .spawn()
            .unwrap();
        let data: Vec<u8> = (0..1 << 20).map(|i| (i % 251) as u8).collect();
        let (mut sent, mut not_ready) = (0, false);
        let start = std::time::Instant::now();
        while sent < data.len() {
            let chunk = &data[sent..data.len().min(sent + 100)];
            let input = p.input().unwrap();
            let n = input.write(chunk);
            sent += n;
            if n < chunk.len() {
                assert_eq!(input.status(), IoStatus::NotReady);
                not_ready = true;
                assert!(start.elapsed().as_secs() < 60, "{sent} bytes taken");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        p.close_input().unwrap();
        let (out, code) = p.read().unwrap();
        assert_eq!(code, 0);
        let out = payload(&out);
        assert!(
            out.len() == data.len() + 4 && out[..4] == *b"got " && out[4..] == data[..],
            "{} bytes echoed of {}",
            out.len(),
            data.len() + 4
        );
        assert!(not_ready, "the pipe never filled");
    }

    /// Writing after a read-ahead that took nothing seeks back to offset 0,
    /// which isn't a failure (upstream takes it for one).
    #[test]
    fn write_after_read_ahead_from_the_start() {
        let dir = crate::test_support::TempDir::new("process-win-seek");
        let path = dir.path("data.txt");
        std::fs::write(&path, b"abc").unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut stream = HandleStream::new(file.into_raw_handle() as HANDLE, "r+b", true);
        // (the read-ahead takes the file, the read none of it)
        assert_eq!(stream.read(&mut []).unwrap(), 0);
        assert_eq!(stream.left, 3);
        assert_eq!(stream.write(b"X").unwrap(), 1);
        stream.close().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"Xbc");
    }

    #[test]
    fn output_not_ready_then_eof() {
        let _lock = lock();
        let mut p = helper("sleep").stdout(ProcessIo::App).spawn().unwrap();
        read_until(&mut p, &[MARK]);
        let mut buf = [0u8; 64];
        assert_eq!(p.output().unwrap().read(&mut buf), 0);
        assert_eq!(p.output().unwrap().status(), IoStatus::NotReady);
        p.kill(true).unwrap();
        assert_eq!(p.wait(true).unwrap(), Some(1));
        let out = p.output().unwrap();
        assert_eq!(out.read(&mut buf), 0);
        assert_eq!(out.status(), IoStatus::Eof);
    }

    #[test]
    fn environment_directory_and_redirect() {
        let _lock = lock();
        let dir = crate::test_support::TempDir::new("process-win");
        let env = Environment::new(true);
        env.set(CHILD_VAR, "env", true).unwrap();
        env.set("SDL_TEST_VALUE", "42", true).unwrap();
        let mut p = helper_with(env)
            .working_directory(&dir.0)
            .stdout(ProcessIo::App)
            .spawn()
            .unwrap();
        let (data, code) = p.read().unwrap();
        assert_eq!(code, 0);
        let text = String::from_utf8(payload(&data).to_vec()).unwrap();
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("42"));
        let cwd = lines.next().unwrap();
        let normalize = |p: &str| p.replace('/', "\\").trim_end_matches('\\').to_lowercase();
        assert_eq!(normalize(cwd), normalize(&dir.0));

        // Standard output and error redirected to a file.
        let path = dir.path("out.txt");
        let file = std::fs::File::create(&path).unwrap();
        let mut p = helper("echo")
            .stdout(ProcessIo::Redirect(file))
            .stderr_to_stdout(true)
            .spawn()
            .unwrap();
        assert_eq!(p.wait(true).unwrap(), Some(0));
        assert_eq!(
            p.read().unwrap_err().to_string(),
            "Process not created with I/O enabled"
        );
        drop(p);
        let data = std::fs::read(&path).unwrap();
        assert_eq!(payload(&data), b"got oops\n");
    }

    #[test]
    fn kill_and_errors() {
        let _lock = lock();
        let mut p = helper("sleep").spawn().unwrap();
        assert_eq!(p.wait(false).unwrap(), None);
        p.kill(true).unwrap();
        assert_eq!(p.wait(true).unwrap(), Some(1));

        let mut p = helper("exit").background(true).spawn().unwrap();
        assert!(p
            .properties()
            .get_bool(PROP_PROCESS_BACKGROUND_BOOLEAN)
            .unwrap());
        assert_eq!(p.wait(true).unwrap(), Some(0));

        assert_eq!(
            Process::create::<&str>(&[], false).unwrap_err().to_string(),
            "Parameter 'args' is invalid"
        );
        assert_eq!(
            ProcessBuilder::new([""]).spawn().unwrap_err().to_string(),
            "Either SDL_PROP_PROCESS_CREATE_ARGS_POINTER or SDL_PROP_PROCESS_CREATE_CMDLINE_STRING must be valid"
        );
        let e = Process::create(&["C:\\nonexistent\\program.exe"], false)
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("CreateProcess: "), "{e}");
    }

    /// The breaks this process got (see [`count_break`]).
    static BREAKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    /// A console control handler that counts the breaks and takes them.
    unsafe extern "system" fn count_break(ctrl_type: u32) -> BOOL {
        if ctrl_type == CTRL_BREAK_EVENT {
            BREAKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return 1;
        }
        0
    }

    /// A child without windows is sent a break, to its own process group:
    /// it ends, and this process, on the same console, gets none (upstream
    /// sends it to every process on the console).
    #[test]
    fn kill_breaks_only_the_child() {
        use windows_sys::Win32::Foundation::STATUS_CONTROL_C_EXIT;
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

        let _lock = lock();
        // SAFETY: the handler only counts; it is removed below.
        assert_ne!(unsafe { SetConsoleCtrlHandler(Some(count_break), 1) }, 0);
        let mut p = helper("sleep").stdout(ProcessIo::App).spawn().unwrap();
        read_until(&mut p, &[MARK]);
        // kill(false) sends the break only when the child takes no WM_CLOSE:
        // when its main thread has no message queue (on the Windows runner
        // it has one, and the sleeping child leaves the WM_CLOSE posted to
        // it unread, as upstream's would).
        let thread_id = p.internal.as_ref().unwrap().process_information.dwThreadId;
        // SAFETY: WM_NULL to the child's main thread does nothing.
        if unsafe { PostThreadMessageW(thread_id, 0, 0, 0) } != 0 {
            p.kill(false).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(500));
            assert_eq!(
                p.wait(false).unwrap(),
                None,
                "the child ended on a WM_CLOSE"
            );
            p.kill(true).unwrap();
            // SAFETY: removes the handler added above.
            unsafe { SetConsoleCtrlHandler(Some(count_break), 0) };
            assert_eq!(BREAKS.load(std::sync::atomic::Ordering::SeqCst), 0);
            eprintln!("note: the child's thread takes a WM_CLOSE: no break is sent");
            return;
        }
        p.kill(false).unwrap();
        let start = std::time::Instant::now();
        let code = loop {
            if let Some(code) = p.wait(false).unwrap() {
                break Some(code);
            }
            if start.elapsed().as_secs() >= 10 {
                // (Wine's GenerateConsoleCtrlEvent() reports a break sent
                // to a process group that it doesn't deliver)
                assert!(crate::core::windows::is_wine(), "the child didn't end");
                p.kill(true).unwrap();
                break None;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        // (a handler runs on a thread of its own: time for one to run)
        std::thread::sleep(std::time::Duration::from_millis(500));
        // SAFETY: removes the handler added above.
        unsafe { SetConsoleCtrlHandler(Some(count_break), 0) };
        assert_eq!(
            BREAKS.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "this process got the break"
        );
        match code {
            // (GenerateConsoleCtrlEvent() fails without a console)
            Some(1) => eprintln!("note: no console to send a break on: the child was terminated"),
            None => eprintln!("note: the break didn't reach the child: it was terminated"),
            Some(code) => assert_eq!(code, STATUS_CONTROL_C_EXIT, "the child's exit code"),
        }
    }

    #[test]
    fn kill_asks_windows_to_close() {
        let _lock = lock();
        // (only once the child has a window to take the WM_CLOSE: before,
        // kill(false) would send it a break)
        let mut p = helper("window").stdout(ProcessIo::App).spawn().unwrap();
        let data = read_until(&mut p, &[b"ready\n", b"nowindow\n"]);
        if data.ends_with(b"nowindow\n") {
            assert_eq!(p.wait(true).unwrap(), Some(98));
            crate::test_support::skip("desktop", "the child could not create a window");
            return;
        }
        p.kill(false).unwrap();
        assert_eq!(p.wait(true).unwrap(), Some(42));
    }
}
