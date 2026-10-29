// Rust translation of src/process/SDL_process.c, SDL_sysprocess.h and
// include/SDL3/SDL_process.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Child processes.
//!
//! A [`ProcessBuilder`] describes the process to start (its arguments,
//! environment, working directory and standard I/O); [`Process`] is the
//! running process. Standard I/O connected to the application
//! ([`ProcessIo::App`]) is exposed as [`IoStream`]s; output streams don't
//! block (a read with nothing available reports [`IoStatus::NotReady`], like
//! upstream's non-blocking pipes).
//!
//! On Windows the backend is upstream's `CreateProcessW()` one
//! (`SDL_windowsprocess.c`); on Emscripten, which has no processes, on
//! Android, where upstream builds the dummy one (`SDL_PROCESS_DUMMY`), and
//! on tvOS, where CMake's `posix_spawn()` check fails, it is the dummy one;
//! everywhere else (iOS and visionOS too) it is built on `std::process`, in
//! place of upstream's `posix_spawn()` backend, which makes it available on
//! every platform with a Rust standard library.
//!
//! [`IoStatus::NotReady`]: crate::io::IoStatus::NotReady

use std::fs::File;

use crate::error::{Error, Result};
use crate::io::IoStream;
use crate::properties::Properties;
use crate::stdlib::Environment;

#[cfg(any(target_os = "android", target_os = "emscripten", target_os = "tvos"))]
mod dummy;
#[cfg(any(target_os = "android", target_os = "emscripten", target_os = "tvos"))]
use dummy as sys;
#[cfg(not(any(
    windows,
    target_os = "android",
    target_os = "emscripten",
    target_os = "tvos"
)))]
mod posix;
#[cfg(not(any(
    windows,
    target_os = "android",
    target_os = "emscripten",
    target_os = "tvos"
)))]
use posix as sys;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

/// the process ID of the process. Translation of `SDL_PROP_PROCESS_PID_NUMBER`.
pub const PROP_PROCESS_PID_NUMBER: &str = "SDL.process.pid";
/// true if the process is running in the background.
/// Translation of `SDL_PROP_PROCESS_BACKGROUND_BOOLEAN`.
pub const PROP_PROCESS_BACKGROUND_BOOLEAN: &str = "SDL.process.background";

/// Description of where standard I/O should be directed when creating a
/// process. Translation of `SDL_ProcessIO`.
///
/// If a standard I/O stream is set to [`ProcessIo::Inherited`], it will go
/// to the same place as the application's I/O stream. This is the default
/// for standard output and standard error.
///
/// If a standard I/O stream is set to [`ProcessIo::Null`], it is connected
/// to `NUL:` on Windows and `/dev/null` on POSIX systems. This is the
/// default for standard input.
///
/// If a standard I/O stream is set to [`ProcessIo::App`], it is connected to
/// a new [`IoStream`] that is available to the application. Standard input
/// will be available as [`Process::input`] and allows the application to
/// write to the process; standard output as [`Process::output`] and allows
/// the application to read from it; standard error as [`Process::error`].
///
/// If a standard I/O stream is set to [`ProcessIo::Redirect`], it is
/// connected to an existing file.
#[derive(Debug, Default)]
pub enum ProcessIo {
    /// The I/O stream is inherited from the application.
    #[default]
    Inherited,
    /// The I/O stream is ignored.
    Null,
    /// The I/O stream is connected to a new IoStream that the application can read or write
    App,
    /// The I/O stream is redirected to an existing file.
    Redirect(File),
}

impl ProcessIo {
    #[cfg_attr(
        any(target_os = "android", target_os = "emscripten", target_os = "tvos"),
        allow(dead_code)
    )] // (by the backends)
    fn is_inherited(&self) -> bool {
        matches!(self, ProcessIo::Inherited)
    }
}

/// The description of a process to create. Translation of the
/// `SDL_PROP_PROCESS_CREATE_*` properties of `SDL_CreateProcessWithProperties()`.
#[derive(Debug)]
pub struct ProcessBuilder {
    args: Vec<String>,
    environment: Option<Environment>,
    working_directory: Option<String>,
    stdin: ProcessIo,
    stdout: ProcessIo,
    stderr: Option<ProcessIo>,
    stderr_to_stdout: bool,
    background: bool,
    cmdline: Option<String>,
}

impl ProcessBuilder {
    /// A process running `args[0]` (looked up in the `PATH` if it has no
    /// path separator) with the arguments `args` (including the program
    /// name). `SDL_PROP_PROCESS_CREATE_ARGS_POINTER`.
    pub fn new<I, S>(args: I) -> ProcessBuilder
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ProcessBuilder {
            args: args.into_iter().map(Into::into).collect(),
            environment: None,
            working_directory: None,
            stdin: ProcessIo::Null,
            stdout: ProcessIo::Inherited,
            stderr: None,
            stderr_to_stdout: false,
            background: false,
            cmdline: None,
        }
    }

    /// The environment for the new process (defaults to the
    /// application's, [`Environment::process`]).
    /// `SDL_PROP_PROCESS_CREATE_ENVIRONMENT_POINTER`.
    pub fn environment(mut self, environment: Environment) -> Self {
        self.environment = Some(environment);
        self
    }

    /// The working directory for the new process.
    /// `SDL_PROP_PROCESS_CREATE_WORKING_DIRECTORY_STRING`.
    pub fn working_directory(mut self, directory: impl Into<String>) -> Self {
        self.working_directory = Some(directory.into());
        self
    }

    /// Where standard input comes from (default [`ProcessIo::Null`]).
    /// `SDL_PROP_PROCESS_CREATE_STDIN_NUMBER`/`_POINTER`.
    pub fn stdin(mut self, io: ProcessIo) -> Self {
        self.stdin = io;
        self
    }

    /// Where standard output goes (default [`ProcessIo::Inherited`]).
    /// `SDL_PROP_PROCESS_CREATE_STDOUT_NUMBER`/`_POINTER`.
    pub fn stdout(mut self, io: ProcessIo) -> Self {
        self.stdout = io;
        self
    }

    /// Where standard error goes (default [`ProcessIo::Inherited`]).
    /// `SDL_PROP_PROCESS_CREATE_STDERR_NUMBER`/`_POINTER`.
    pub fn stderr(mut self, io: ProcessIo) -> Self {
        self.stderr = Some(io);
        self
    }

    /// Send standard error to the same place as standard output (ignored
    /// if [`stderr`](Self::stderr) is set).
    /// `SDL_PROP_PROCESS_CREATE_STDERR_TO_STDOUT_BOOLEAN`.
    pub fn stderr_to_stdout(mut self, redirect: bool) -> Self {
        self.stderr_to_stdout = redirect;
        self
    }

    /// Run the process in the background: its standard I/O that would be
    /// inherited goes nowhere, and its exit code reads as 0.
    /// `SDL_PROP_PROCESS_CREATE_BACKGROUND_BOOLEAN`.
    pub fn background(mut self, background: bool) -> Self {
        self.background = background;
        self
    }

    /// A string containing the program to run and any parameters, passed
    /// directly to `CreateProcess` on Windows (in place of the arguments,
    /// which may then be empty); does nothing on other platforms. Only
    /// important to start programs that do non-standard command-line
    /// processing. `SDL_PROP_PROCESS_CREATE_CMDLINE_STRING`.
    pub fn cmdline(mut self, cmdline: impl Into<String>) -> Self {
        self.cmdline = Some(cmdline.into());
        self
    }

    /// Create the process. Translation of `SDL_CreateProcessWithProperties()`.
    pub fn spawn(self) -> Result<Process> {
        #[cfg(windows)]
        if self.args.first().is_none_or(|a| a.is_empty())
            && self.cmdline.as_deref().is_none_or(str::is_empty)
        {
            return Err(Error::new("Either SDL_PROP_PROCESS_CREATE_ARGS_POINTER or SDL_PROP_PROCESS_CREATE_CMDLINE_STRING must be valid"));
        }
        #[cfg(not(windows))]
        if self.args.first().is_none_or(|a| a.is_empty()) {
            return Err(Error::invalid_param("SDL_PROP_PROCESS_CREATE_ARGS_POINTER"));
        }

        let background = self.background;
        let props = Properties::new();
        props.set(PROP_PROCESS_BACKGROUND_BOOLEAN, background)?;

        let mut process = Process {
            alive: false,
            background,
            exitcode: 0,
            props,
            stdin: None,
            stdout: None,
            stderr: None,
            internal: None,
        };
        sys::sys_create_process(&mut process, self)?;
        process.alive = true;
        Ok(process)
    }
}

/// A running (or exited) process. Translation of `SDL_Process *`; dropping
/// it is `SDL_DestroyProcess()`, which closes the I/O streams (a process
/// still running keeps running).
#[derive(Debug)]
pub struct Process {
    alive: bool,
    background: bool,
    exitcode: i32,
    props: Properties,
    stdin: Option<IoStream<'static>>,
    stdout: Option<IoStream<'static>>,
    stderr: Option<IoStream<'static>>,
    // (last, so that it goes after the streams)
    // (the dummy backend has none)
    #[cfg_attr(
        any(target_os = "android", target_os = "emscripten", target_os = "tvos"),
        allow(dead_code)
    )]
    internal: Option<sys::ProcessData>,
}

impl Process {
    /// Create a new process, with standard input and output connected to
    /// the application if `pipe_stdio` (and inherited standard error).
    /// Translation of `SDL_CreateProcess()`.
    pub fn create<S: AsRef<str>>(args: &[S], pipe_stdio: bool) -> Result<Process> {
        if args.first().is_none_or(|a| a.as_ref().is_empty()) {
            return Err(Error::invalid_param("args"));
        }

        let mut builder = ProcessBuilder::new(args.iter().map(|a| a.as_ref().to_string()));
        if pipe_stdio {
            builder = builder.stdin(ProcessIo::App).stdout(ProcessIo::App);
        }
        builder.spawn()
    }

    /// The properties of the process. Translation of `SDL_GetProcessProperties()`.
    pub fn properties(&self) -> Properties {
        self.props.clone()
    }

    /// Read all the output from the process (waiting for it to exit), and
    /// its exit code. Translation of `SDL_ReadProcess()`.
    pub fn read(&mut self) -> Result<(Vec<u8>, i32)> {
        let Some(io) = &mut self.stdout else {
            return Err(Error::new("Process not created with I/O enabled"));
        };

        let result = io.load_all()?;

        let exitcode = self.wait(true)?.unwrap_or(-1);

        Ok((result, exitcode))
    }

    /// The stream to write to the process's standard input.
    /// Translation of `SDL_GetProcessInput()`.
    pub fn input(&mut self) -> Result<&mut IoStream<'static>> {
        self.stdin
            .as_mut()
            .ok_or_else(|| Error::new("Process not created with standard input available"))
    }

    /// Close the process's standard input (so it sees the end of its
    /// input), as `SDL_CloseIO(SDL_GetProcessInput())` does.
    pub fn close_input(&mut self) -> Result<()> {
        match self.stdin.take() {
            Some(io) => io.close(),
            None => Err(Error::new(
                "Process not created with standard input available",
            )),
        }
    }

    /// The stream to read the process's standard output from.
    /// Translation of `SDL_GetProcessOutput()`.
    pub fn output(&mut self) -> Result<&mut IoStream<'static>> {
        self.stdout
            .as_mut()
            .ok_or_else(|| Error::new("Process not created with standard output available"))
    }

    /// The stream to read the process's standard error from
    /// (`SDL_PROP_PROCESS_STDERR_POINTER`).
    pub fn error(&mut self) -> Result<&mut IoStream<'static>> {
        self.stderr
            .as_mut()
            .ok_or_else(|| Error::new("Process not created with standard error available"))
    }

    /// Stop the process: forcibly if `force`, otherwise asking it to stop
    /// (`SIGTERM` on POSIX, `WM_CLOSE` or a console break on Windows).
    /// Translation of `SDL_KillProcess()`.
    pub fn kill(&mut self, force: bool) -> Result<()> {
        if !self.alive {
            return Err(Error::new("Process isn't running"));
        }

        sys::sys_kill_process(self, force)
    }

    /// Wait for the process to finish (or, without `block`, check whether
    /// it has): `Ok(Some(exitcode))` once it has exited, `Ok(None)` if it is
    /// still running. The exit code is negative for a process killed by a
    /// signal (the signal number), and 0 for a background process.
    /// Translation of `SDL_WaitProcess()`.
    pub fn wait(&mut self, block: bool) -> Result<Option<i32>> {
        if !self.alive {
            return Ok(Some(self.exitcode));
        }

        match sys::sys_wait_process(self, block)? {
            Some(exitcode) => {
                self.exitcode = exitcode;
                self.alive = false;
                if self.background {
                    self.exitcode = 0;
                }
                Ok(Some(self.exitcode))
            }
            None => Ok(None),
        }
    }
}

impl Drop for Process {
    /// Translation of `SDL_DestroyProcess()`.
    fn drop(&mut self) {
        // Check to see if the process has exited, will reap zombies on POSIX platforms
        if self.alive {
            let _ = self.wait(false);
        }

        sys::sys_destroy_process(self);
    }
}
