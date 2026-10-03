// Rust translation of src/process/SDL_process.c, SDL_sysprocess.h,
// src/process/posix/SDL_posixprocess.c and include/SDL3/SDL_process.h from
// Simple DirectMedia Layer.
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
//! The backend is built on `std::process`, in place of upstream's
//! `posix_spawn()` (POSIX) and `CreateProcess()` (Windows) backends, which
//! makes it available on every platform with a Rust standard library.
//! Differences: a background process is started in its own process group
//! (upstream detaches it with `setsid()` in a double fork) and remains a
//! child of the application, and writes to an input stream block until the
//! process reads them.

use std::collections::VecDeque;
use std::fs::File;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::error::{Error, Result};
use crate::io::{IoInterface, IoStatus, IoStop, IoStream};
use crate::properties::Properties;
use crate::stdlib::Environment;

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

    /// Create the process. Translation of `SDL_CreateProcessWithProperties()`.
    pub fn spawn(self) -> Result<Process> {
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
            child: None,
            stdin: None,
            stdout: None,
            stderr: None,
        };
        sys_create_process(&mut process, self)?;
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
    child: Option<Child>,
    stdin: Option<IoStream<'static>>,
    stdout: Option<IoStream<'static>>,
    stderr: Option<IoStream<'static>>,
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
    /// (`SIGTERM` on POSIX). Translation of `SDL_KillProcess()`.
    pub fn kill(&mut self, force: bool) -> Result<()> {
        if !self.alive {
            return Err(Error::new("Process isn't running"));
        }

        sys_kill_process(self, force)
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

        match sys_wait_process(self, block)? {
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

        sys_destroy_process(self);
    }
}

/// The output side of a pipe from the process: a thread reads the pipe so
/// that reads of the stream don't block (upstream's `O_NONBLOCK`).
struct PipeOutput {
    shared: Arc<Mutex<PipeBuffer>>,
}

#[derive(Default)]
struct PipeBuffer {
    data: VecDeque<u8>,
    eof: bool,
    error: Option<String>,
}

impl PipeOutput {
    fn new(mut reader: impl std::io::Read + Send + 'static) -> PipeOutput {
        let shared = Arc::new(Mutex::new(PipeBuffer::default()));
        let filler = shared.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                let result = reader.read(&mut chunk);
                let mut buffer = filler.lock().unwrap_or_else(|e| e.into_inner());
                match result {
                    Ok(0) => {
                        buffer.eof = true;
                        break;
                    }
                    Ok(n) => buffer.data.extend(&chunk[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        buffer.error = Some(e.to_string());
                        buffer.eof = true;
                        break;
                    }
                }
            }
        });
        PipeOutput { shared }
    }
}

impl IoInterface for PipeOutput {
    fn can_read(&self) -> bool {
        true
    }

    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let mut buffer = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let n = buf.len().min(buffer.data.len());
        for (dst, src) in buf.iter_mut().zip(buffer.data.drain(..n)) {
            *dst = src;
        }
        if n > 0 || buf.is_empty() {
            return Ok(n);
        }
        if let Some(e) = &buffer.error {
            return Err(IoStop {
                bytes: 0,
                status: IoStatus::Error,
                error: Some(Error::new(e.clone())),
            });
        }
        if buffer.eof {
            Err(IoStop::status(0, IoStatus::Eof))
        } else {
            Err(IoStop::status(0, IoStatus::NotReady))
        }
    }
}

/// The input side of a pipe to the process.
struct PipeInput {
    writer: Option<std::io::PipeWriter>,
}

impl IoInterface for PipeInput {
    fn can_write(&self) -> bool {
        true
    }

    fn write(&mut self, buf: &[u8]) -> std::result::Result<usize, IoStop> {
        use std::io::Write;
        let Some(writer) = &mut self.writer else {
            return Err(IoStop::status(0, IoStatus::Error));
        };
        loop {
            match writer.write(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    return Err(IoStop {
                        bytes: 0,
                        status: IoStatus::Error,
                        error: Some(Error::new(io_error_message(&e))),
                    })
                }
            }
        }
    }

    fn flush(&mut self) -> std::result::Result<(), IoStop> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.writer = None;
        Ok(())
    }
}

/// An OS error's message without Rust's " (os error N)" suffix (the
/// `strerror()` text upstream reports).
fn io_error_message(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.rfind(" (os error ") {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

fn pipe() -> Result<(std::io::PipeReader, std::io::PipeWriter)> {
    std::io::pipe().map_err(|e| Error::new(format!("pipe() failed: {}", io_error_message(&e))))
}

/// The child's end of a standard I/O stream, and the application's end of
/// an [`ProcessIo::App`] pipe.
enum AppEnd {
    None,
    Reader(std::io::PipeReader),
    Writer(std::io::PipeWriter),
}

fn child_stdio(io: ProcessIo, input: bool) -> Result<(Stdio, AppEnd)> {
    Ok(match io {
        ProcessIo::Inherited => (Stdio::inherit(), AppEnd::None),
        ProcessIo::Null => (Stdio::null(), AppEnd::None),
        ProcessIo::App => {
            let (reader, writer) = pipe()?;
            if input {
                (Stdio::from(reader), AppEnd::Writer(writer))
            } else {
                (Stdio::from(writer), AppEnd::Reader(reader))
            }
        }
        ProcessIo::Redirect(file) => (Stdio::from(file), AppEnd::None),
    })
}

/// Translation of `SDL_SYS_CreateProcessWithProperties()`.
fn sys_create_process(process: &mut Process, builder: ProcessBuilder) -> Result<()> {
    let ProcessBuilder {
        args,
        environment,
        working_directory,
        mut stdin,
        mut stdout,
        stderr,
        stderr_to_stdout,
        background,
    } = builder;
    let redirect_stderr = stderr_to_stdout && stderr.is_none();
    let mut stderr = stderr.unwrap_or_default();
    let env = environment.unwrap_or_else(Environment::process);

    let mut command = Command::new(&args[0]);
    command.args(&args[1..]);
    command.env_clear();
    for variable in env.variables() {
        if let Some((name, value)) = variable.split_once('=') {
            command.env(name, value);
        }
    }
    if let Some(directory) = &working_directory {
        command.current_dir(directory);
    }

    // Background processes don't have access to the terminal
    if background {
        if stdin.is_inherited() {
            stdin = ProcessIo::Null;
        }
        if stdout.is_inherited() {
            stdout = ProcessIo::Null;
        }
        if stderr.is_inherited() {
            stderr = ProcessIo::Null;
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // (detach from the terminal's process group)
            command.process_group(0);
        }
    }

    let (stdin_stdio, stdin_end) = child_stdio(stdin, true)?;
    command.stdin(stdin_stdio);

    if redirect_stderr {
        // Standard error goes wherever standard output goes
        match stdout {
            ProcessIo::App => {
                let (reader, writer) = pipe()?;
                let writer2 = writer
                    .try_clone()
                    .map_err(|e| Error::new(format!("dup() failed: {}", io_error_message(&e))))?;
                command.stdout(Stdio::from(writer));
                command.stderr(Stdio::from(writer2));
                spawn(
                    process,
                    command,
                    stdin_end,
                    AppEnd::Reader(reader),
                    AppEnd::None,
                )
            }
            ProcessIo::Redirect(file) => {
                let file2 = file
                    .try_clone()
                    .map_err(|e| Error::new(format!("dup() failed: {}", io_error_message(&e))))?;
                command.stdout(Stdio::from(file));
                command.stderr(Stdio::from(file2));
                spawn(process, command, stdin_end, AppEnd::None, AppEnd::None)
            }
            ProcessIo::Null => {
                command.stdout(Stdio::null());
                command.stderr(Stdio::null());
                spawn(process, command, stdin_end, AppEnd::None, AppEnd::None)
            }
            ProcessIo::Inherited => {
                command.stdout(Stdio::inherit());
                command.stderr(Stdio::from(std::io::stdout()));
                spawn(process, command, stdin_end, AppEnd::None, AppEnd::None)
            }
        }
    } else {
        let (stdout_stdio, stdout_end) = child_stdio(stdout, false)?;
        command.stdout(stdout_stdio);
        let (stderr_stdio, stderr_end) = child_stdio(stderr, false)?;
        command.stderr(stderr_stdio);
        spawn(process, command, stdin_end, stdout_end, stderr_end)
    }
}

fn spawn(
    process: &mut Process,
    mut command: Command,
    stdin: AppEnd,
    stdout: AppEnd,
    stderr: AppEnd,
) -> Result<()> {
    // Spawn the new process
    let child = command
        .spawn()
        .map_err(|e| Error::new(format!("posix_spawn() failed: {}", io_error_message(&e))))?;
    // (the child's ends of the pipes close with `command`)
    drop(command);
    process
        .props
        .set(PROP_PROCESS_PID_NUMBER, i64::from(child.id()))?;
    process.child = Some(child);

    if let AppEnd::Writer(writer) = stdin {
        process.stdin = Some(IoStream::open(PipeInput {
            writer: Some(writer),
        }));
    }
    if let AppEnd::Reader(reader) = stdout {
        process.stdout = Some(IoStream::open(PipeOutput::new(reader)));
    }
    if let AppEnd::Reader(reader) = stderr {
        process.stderr = Some(IoStream::open(PipeOutput::new(reader)));
    }
    Ok(())
}

#[cfg(unix)]
mod signal {
    // The C library's kill(2), to ask a process to terminate (std's
    // Child::kill always sends SIGKILL).
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }

    const SIGTERM: i32 = 15;

    pub(super) fn terminate(pid: u32) -> std::io::Result<()> {
        // SAFETY: kill() takes plain integers and has no memory effects.
        let ret = unsafe { kill(pid as i32, SIGTERM) };
        if ret == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

/// Translation of `SDL_SYS_KillProcess()`.
fn sys_kill_process(process: &mut Process, force: bool) -> Result<()> {
    let Some(child) = &mut process.child else {
        return Err(Error::new("Process isn't running"));
    };
    #[cfg(unix)]
    let result = if force {
        child.kill()
    } else {
        signal::terminate(child.id())
    };
    #[cfg(not(unix))]
    let result = {
        let _ = force;
        child.kill()
    };
    result.map_err(|e| Error::new(format!("Could not kill(): {}", io_error_message(&e))))
}

/// Translation of `SDL_SYS_WaitProcess()`.
fn sys_wait_process(process: &mut Process, block: bool) -> Result<Option<i32>> {
    let Some(child) = &mut process.child else {
        return Ok(Some(-255));
    };
    let status = if block {
        child.wait().map(Some)
    } else {
        child.try_wait()
    }
    .map_err(|e| Error::new(format!("Could not waitpid(): {}", io_error_message(&e))))?;

    let Some(status) = status else {
        return Ok(None);
    };

    if process.background {
        return Ok(Some(0));
    }

    if let Some(code) = status.code() {
        return Ok(Some(code));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return Ok(Some(-signal));
        }
    }
    Ok(Some(-255))
}

/// Translation of `SDL_SYS_DestroyProcess()`.
fn sys_destroy_process(process: &mut Process) {
    if let Some(io) = process.stdin.take() {
        let _ = io.close();
    }
    if let Some(io) = process.stdout.take() {
        let _ = io.close();
    }
    if let Some(io) = process.stderr.take() {
        let _ = io.close();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn read_output_and_exit_code() {
        let mut p = Process::create(&["sh", "-c", "echo hello; exit 3"], true).unwrap();
        assert!(p.properties().get_number(PROP_PROCESS_PID_NUMBER).unwrap() > 0);
        let (data, code) = p.read().unwrap();
        assert_eq!(data, b"hello\n");
        assert_eq!(code, 3);
        assert_eq!(p.wait(false).unwrap(), Some(3));
        assert_eq!(
            p.kill(true).unwrap_err().to_string(),
            "Process isn't running"
        );
    }

    #[test]
    fn input_and_stderr() {
        let mut p = ProcessBuilder::new(["sh", "-c", "read x; echo got $x; echo oops >&2"])
            .stdin(ProcessIo::App)
            .stdout(ProcessIo::App)
            .stderr_to_stdout(true)
            .spawn()
            .unwrap();
        assert_eq!(p.input().unwrap().write(b"abc\n"), 4);
        p.close_input().unwrap();
        let (data, code) = p.read().unwrap();
        assert_eq!(data, b"got abc\noops\n");
        assert_eq!(code, 0);
    }

    #[test]
    fn environment_and_directory() {
        let env = Environment::new(false);
        env.set("SDL_TEST_VALUE", "42", true).unwrap();
        let mut p = ProcessBuilder::new(["/bin/sh", "-c", "echo $SDL_TEST_VALUE; pwd"])
            .environment(env)
            .working_directory("/")
            .stdout(ProcessIo::App)
            .spawn()
            .unwrap();
        assert_eq!(p.read().unwrap(), (b"42\n/\n".to_vec(), 0));
    }

    #[test]
    fn kill_and_errors() {
        let mut p = Process::create(&["sleep", "10"], false).unwrap();
        assert_eq!(p.wait(false).unwrap(), None);
        p.kill(false).unwrap();
        assert_eq!(p.wait(true).unwrap(), Some(-15));
        assert_eq!(
            p.read().unwrap_err().to_string(),
            "Process not created with I/O enabled"
        );

        assert_eq!(
            Process::create::<&str>(&[], false).unwrap_err().to_string(),
            "Parameter 'args' is invalid"
        );
        assert!(Process::create(&["/nonexistent/program"], false)
            .unwrap_err()
            .to_string()
            .starts_with("posix_spawn() failed: "));

        let mut p = ProcessBuilder::new(["sh", "-c", "exit 5"])
            .background(true)
            .spawn()
            .unwrap();
        assert_eq!(p.wait(true).unwrap(), Some(0));
    }
}
