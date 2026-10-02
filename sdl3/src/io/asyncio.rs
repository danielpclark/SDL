// Rust translation of src/io/SDL_asyncio.c and src/io/generic/SDL_asyncio_generic.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Asynchronous file I/O. Translation of `SDL_asyncio.h`.
//!
//! Requests are started with [`AsyncIo::read`], [`AsyncIo::write`] and
//! [`AsyncIo::close`], and finish later as [`AsyncIoOutcome`]s collected
//! from an [`AsyncIoQueue`]. Buffers are moved into a request and handed
//! back in its outcome, so no memory is shared with the I/O threads while
//! a request is in flight.
//!
//! This is the generic backend: it uses a threadpool to block on synchronous
//! i/o. This is not ideal, it's meant to be used if there isn't a
//! platform-specific backend that can do something more efficient!

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use super::{IoStatus, IoStream, IoWhence};
use crate::error::{Error, Result};
use crate::thread::{InitState, Thread};

/// Types of asynchronous I/O tasks. Translation of `SDL_AsyncIOTaskType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AsyncIoTaskType {
    /// A read operation.
    Read,
    /// A write operation.
    Write,
    /// A close operation.
    Close,
}

/// Possible outcomes of an asynchronous I/O task. Translation of `SDL_AsyncIOResult`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AsyncIoResult {
    /// request was completed without error
    Complete,
    /// request failed for some reason; check the error for details
    Failure,
    /// request was canceled before completing.
    Canceled,
}

/// Information about a completed asynchronous I/O request.
/// Translation of `SDL_AsyncIOOutcome`.
#[derive(Debug)]
pub struct AsyncIoOutcome<U> {
    /// what generated this task. `None` for tasks from [`load_file_async`].
    pub asyncio: Option<AsyncIo>,
    /// What sort of task was this? Read, write, etc?
    pub kind: AsyncIoTaskType,
    /// the result of the work (success, failure, cancellation).
    pub result: AsyncIoResult,
    /// buffer where data was read/written. Empty for close tasks.
    pub buffer: Vec<u8>,
    /// offset in the [`AsyncIo`] where data was read/written.
    pub offset: u64,
    /// number of bytes the task was to read/write.
    pub bytes_requested: u64,
    /// actual number of bytes that were read/written.
    pub bytes_transferred: u64,
    /// the value passed in when the request was started.
    pub userdata: U,
}

impl<U> AsyncIoOutcome<U> {
    /// The bytes actually transferred: `buffer[..bytes_transferred]`.
    pub fn data(&self) -> &[u8] {
        let n = usize::try_from(self.bytes_transferred).unwrap_or(usize::MAX);
        &self.buffer[..n.min(self.buffer.len())]
    }
}

/// Translation of `SDL_AsyncIOTask`.
struct AsyncIoTask<U> {
    asyncio: AsyncIo,
    kind: AsyncIoTaskType,
    queue: Arc<QueueInner<U>>,
    offset: u64,
    flush: bool,
    buffer: Vec<u8>,
    result: AsyncIoResult,
    requested_size: u64,
    result_size: u64,
    app_userdata: U,
}

/// A task with its queue type erased, as the threadpool and a pending close
/// see it.
trait ErasedTask: Send {
    /// `SynchronousIO()`.
    fn run(self: Box<Self>);
    /// Mark the task canceled and complete it.
    fn cancel(self: Box<Self>);
    /// The `tasks_inflight` counter of the task's queue.
    fn inflight(&self) -> &AtomicI32;
}

impl<U: Send + 'static> ErasedTask for AsyncIoTask<U> {
    fn run(self: Box<Self>) {
        synchronous_io(*self);
    }
    fn cancel(mut self: Box<Self>) {
        self.result = AsyncIoResult::Canceled;
        async_io_task_complete(*self);
    }
    fn inflight(&self) -> &AtomicI32 {
        &self.queue.tasks_inflight
    }
}

/// The close request, which isn't queued until all pending work for this
/// file is done (`SDL_AsyncIO::closing`).
enum Closing {
    Open,
    Pending(Box<dyn ErasedTask>),
    Queued,
}

/// Translation of the bookkeeping fields of `SDL_AsyncIO`, under its `lock`.
struct AsyncState {
    /// How many tasks are linked into `SDL_AsyncIO::tasks`.
    tasks: usize,
    closing: Closing,
}

/// Translation of `SDL_AsyncIO` with `GenericAsyncIOData`.
struct AsyncIoInner {
    state: Mutex<AsyncState>,
    /// `GenericAsyncIOData::lock` and `io`: the stream, until the close
    /// task runs.
    ///
    /// !!! FIXME: we can skip this lock if we have an equivalent of pread/pwrite
    io: Mutex<Option<IoStream<'static>>>,
    /// true if this is a [`load_file_async`] open.
    oneshot: bool,
    /// true if this file is opened read-only.
    #[allow(dead_code)]
    readonly: bool,
}

/// An asynchronous I/O stream handle. Translation of `SDL_AsyncIO`.
///
/// Cloning gives another handle to the same stream; handles compare equal
/// when they refer to the same stream. The file is closed by
/// [`AsyncIo::close`], or when the last handle and request are gone.
#[derive(Clone)]
pub struct AsyncIo {
    inner: Arc<AsyncIoInner>,
}

impl PartialEq for AsyncIo {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}
impl Eq for AsyncIo {}

impl std::fmt::Debug for AsyncIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncIo")
            .field("id", &Arc::as_ptr(&self.inner))
            .finish_non_exhaustive()
    }
}

/// Translation of `GenericAsyncIOQueueData`, plus `tasks_inflight`.
struct QueueInner<U> {
    /// `completed_tasks`
    completed: Mutex<Vec<AsyncIoTask<U>>>,
    condition: Condvar,
    tasks_inflight: AtomicI32,
}

/// A queue of completed asynchronous I/O tasks. Translation of `SDL_AsyncIOQueue`.
///
/// `U` is the type of the per-request value handed back in each
/// [`AsyncIoOutcome`] (the C `userdata` pointer). Dropping the queue blocks
/// until every request sent to it has completed (`SDL_DestroyAsyncIOQueue()`).
/// It may be shared between threads, several of which can wait on it.
pub struct AsyncIoQueue<U = ()> {
    inner: Arc<QueueInner<U>>,
}

impl<U> std::fmt::Debug for AsyncIoQueue<U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncIoQueue")
            .field(
                "tasks_inflight",
                &self.inner.tasks_inflight.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `AsyncIOTaskComplete()`.
fn async_io_task_complete<U>(task: AsyncIoTask<U>) {
    let data = task.queue.clone();
    let mut completed = lock(&data.completed);
    // (LINKED_LIST_PREPEND, and results are taken from the front: newest first)
    completed.push(task);
    data.condition.notify_one(); // wake a thread waiting on the queue.
}

/// Translation of `SynchronousIO()`: synchronous i/o is offloaded onto the
/// threadpool. This function does the threaded work.
fn synchronous_io<U>(mut task: AsyncIoTask<U>) {
    crate::sdl_assert!(task.result != AsyncIoResult::Canceled); // shouldn't have gotten in here if canceled!

    let size = task.requested_size as usize;

    // this seek won't work if two tasks are reading from the same file at the same time,
    // so we lock here. This makes multiple reads from a single file serialize, but different
    // files will still run in parallel. An app can also open the same file twice to avoid this.
    {
        let mut io = lock(&task.asyncio.inner.io);
        if task.kind == AsyncIoTaskType::Close {
            let okay = match io.take() {
                Some(mut stream) => {
                    let mut okay = true;
                    if task.flush {
                        okay = stream.flush().is_ok();
                    }
                    stream.close().is_ok() && okay
                }
                None => false,
            };
            task.result = if okay {
                AsyncIoResult::Complete
            } else {
                AsyncIoResult::Failure
            };
        } else {
            let seeked = match io.as_mut() {
                Some(stream) => match stream.seek(task.offset as i64, IoWhence::Set) {
                    Ok(_) => Some(stream),
                    Err(_) => None,
                },
                None => None,
            };
            match seeked {
                None => task.result = AsyncIoResult::Failure,
                Some(stream) => {
                    let writing = task.kind == AsyncIoTaskType::Write;
                    task.result_size = if writing {
                        stream.write(&task.buffer[..size])
                    } else {
                        stream.read(&mut task.buffer[..size])
                    } as u64;
                    if task.result_size == task.requested_size {
                        task.result = AsyncIoResult::Complete;
                    } else if writing {
                        task.result = AsyncIoResult::Failure; // it's always a failure on short writes.
                    } else {
                        let status = stream.status();
                        crate::sdl_assert!(status != IoStatus::Ready); // this should have either failed or been EOF.
                        crate::sdl_assert!(status != IoStatus::NotReady); // these should not be non-blocking reads!
                        task.result = if status == IoStatus::Eof {
                            AsyncIoResult::Complete
                        } else {
                            AsyncIoResult::Failure
                        };
                    }
                }
            }
        }
    }

    async_io_task_complete(task);
}

/// Translation of the generic backend's threadpool globals.
struct Threadpool {
    stop_threadpool: bool,
    threadpool_tasks: Vec<Box<dyn ErasedTask>>,
    max_threadpool_threads: i32,
    running_threadpool_threads: i32,
    idle_threadpool_threads: i32,
    threadpool_threads_spun: i32,
}

static THREADPOOL_INIT: InitState = InitState::new();
static THREADPOOL_LOCK: Mutex<Threadpool> = Mutex::new(Threadpool {
    stop_threadpool: false,
    threadpool_tasks: Vec::new(),
    max_threadpool_threads: 0,
    running_threadpool_threads: 0,
    idle_threadpool_threads: 0,
    threadpool_threads_spun: 0,
});
static THREADPOOL_CONDITION: Condvar = Condvar::new();

/// Translation of `AsyncIOThreadpoolWorker()`.
fn async_io_threadpool_worker() -> i32 {
    let mut pool = lock(&THREADPOOL_LOCK);

    while !pool.stop_threadpool {
        // (LINKED_LIST_START of a list built by prepending: the newest task)
        let Some(task) = pool.threadpool_tasks.pop() else {
            // if we go 30 seconds without a new task, terminate unless we're the only thread left.
            pool.idle_threadpool_threads += 1;
            let (guard, rc) = THREADPOOL_CONDITION
                .wait_timeout(pool, Duration::from_millis(30000))
                .unwrap_or_else(|e| e.into_inner());
            pool = guard;
            pool.idle_threadpool_threads -= 1;

            if rc.timed_out() {
                // decide if we have too many idle threads, and if so, quit to let thread pool shrink when not busy.
                if pool.idle_threadpool_threads != 0 {
                    break;
                }
            }

            continue;
        };

        drop(pool);

        // bookkeeping is done, so we drop the mutex and fire the work.
        task.run();

        pool = lock(&THREADPOOL_LOCK); // take the lock again and see if there's another task (if not, we'll wait on the Condition).
    }

    pool.running_threadpool_threads -= 1;

    // this is kind of a hack, but this lets us reuse threadpool_condition to block on shutdown until all threads have exited.
    if pool.stop_threadpool {
        THREADPOOL_CONDITION.notify_all();
    }

    0
}

/// Translation of `MaybeSpinNewWorkerThread()`.
fn maybe_spin_new_worker_thread(pool: &mut Threadpool) -> bool {
    // if all existing threads are busy and the pool of threads isn't maxed out, make a new one.
    if pool.idle_threadpool_threads == 0
        && pool.running_threadpool_threads < pool.max_threadpool_threads
    {
        let threadname = format!("SDLasyncio{}", pool.threadpool_threads_spun);
        let Ok(thread) = Thread::spawn(threadname, async_io_threadpool_worker) else {
            return false;
        };
        thread.detach(); // these terminate themselves when idle too long, so we never WaitThread.
        pool.running_threadpool_threads += 1;
        pool.threadpool_threads_spun += 1;
    }
    true
}

/// Translation of `QueueAsyncIOTask()`.
fn queue_async_io_task(task: Box<dyn ErasedTask>) {
    let mut pool = lock(&THREADPOOL_LOCK);

    if pool.stop_threadpool {
        // just in case.
        drop(pool);
        task.cancel();
    } else {
        pool.threadpool_tasks.push(task);
        maybe_spin_new_worker_thread(&mut pool); // okay if this fails or the thread pool is maxed out. Something will get there eventually.

        // tell idle threads to get to work.
        // This is a broadcast because we want someone from the thread pool to wake up, but
        // also shutdown might also be blocking on this. One of the threads will grab
        // it, the others will go back to sleep.
        THREADPOOL_CONDITION.notify_all();
    }
}

/// Translation of `PrepareThreadpool()`.
///
/// We don't initialize async i/o at all until it's used, so
///  JUST IN CASE two things try to start at the same time,
///  this will make sure everything gets the same mutex.
fn prepare_threadpool() -> Result<()> {
    let mut okay = true;
    if THREADPOOL_INIT.should_init() {
        let mut pool = lock(&THREADPOOL_LOCK);
        pool.max_threadpool_threads = (crate::cpuinfo::num_logical_cpu_cores() * 2) + 1; // !!! FIXME: this should probably have a hint to override.
        pool.max_threadpool_threads = pool.max_threadpool_threads.clamp(1, 8); // 8 is probably more than enough.

        okay = maybe_spin_new_worker_thread(&mut pool); // make sure at least one thread is going, since we'll need it.
        drop(pool);

        THREADPOOL_INIT.set_initialized(okay);
    }
    if okay {
        Ok(())
    } else {
        Err(Error::new("Couldn't create thread"))
    }
}

/// Translation of `ShutdownThreadpool()`.
fn shutdown_threadpool() {
    if THREADPOOL_INIT.should_quit() {
        let mut pool = lock(&THREADPOOL_LOCK);

        // cancel anything that's still pending.
        let pending = std::mem::take(&mut pool.threadpool_tasks);
        for task in pending.into_iter().rev() {
            task.cancel();
        }

        pool.stop_threadpool = true;
        THREADPOOL_CONDITION.notify_all(); // tell the whole threadpool to wake up and quit.

        while pool.running_threadpool_threads > 0 {
            // each threadpool thread will broadcast this condition before it terminates if stop_threadpool is set.
            // we can't just join the threads because they are detached, so the thread pool can automatically shrink as necessary.
            pool = THREADPOOL_CONDITION
                .wait(pool)
                .unwrap_or_else(|e| e.into_inner());
        }

        pool.max_threadpool_threads = 0;
        pool.running_threadpool_threads = 0;
        pool.idle_threadpool_threads = 0;
        pool.threadpool_threads_spun = 0;

        pool.stop_threadpool = false;
        drop(pool);
        THREADPOOL_INIT.set_initialized(false);
    }
}

/// Translation of `AsyncFileModeValid()`: the mode with 'b' added, and
/// whether it is read-only.
fn async_file_mode_valid(mode: &str) -> Option<(&'static str, bool)> {
    const MODE_MAP: [(&str, &str, bool); 4] = [
        ("r", "rb", true),
        ("w", "wb", false),
        ("r+", "r+b", false),
        ("w+", "w+b", false),
    ];
    MODE_MAP
        .iter()
        .find(|(valid, _, _)| *valid == mode)
        .map(|&(_, with_binary, readonly)| (with_binary, readonly))
}

impl AsyncIo {
    /// Use this function to create a new [`AsyncIo`] object for reading
    /// from and/or writing to a named file. Translation of `SDL_AsyncIOFromFile()`.
    ///
    /// The `mode` string understands the following values:
    ///
    /// - "r": Open a file for reading only. It must exist.
    /// - "w": Open a file for writing only. It will create missing files or
    ///   truncate existing ones.
    /// - "r+": Open a file for update both reading and writing. The file must
    ///   exist.
    /// - "w+": Create an empty file for both reading and writing. If a file
    ///   with the same name already exists its content is erased and the file
    ///   is treated as a new empty file.
    ///
    /// There is no "b" mode, as there is only "binary" style I/O, and no
    /// "a" mode for appending, since you specify the position when starting
    /// a task.
    ///
    /// This function supports Unicode filenames, but they must be encoded in
    /// UTF-8 format, regardless of the underlying operating system.
    ///
    /// This call is _not_ asynchronous; it will open the file before
    /// returning, under the assumption that doing so is generally a fast
    /// operation. Future reads and writes to the opened file will be async,
    /// however.
    pub fn from_file(file: &str, mode: &str) -> Result<AsyncIo> {
        AsyncIo::open(file, mode, false)
    }

    fn open(file: &str, mode: &str, oneshot: bool) -> Result<AsyncIo> {
        let Some((binary_mode, readonly)) = async_file_mode_valid(mode) else {
            return Err(Error::new("Unsupported file mode"));
        };

        // SDL_SYS_AsyncIOFromFile_Generic()
        prepare_threadpool()?;
        let io = IoStream::from_file(file, binary_mode)?;

        Ok(AsyncIo {
            inner: Arc::new(AsyncIoInner {
                state: Mutex::new(AsyncState {
                    tasks: 0,
                    closing: Closing::Open,
                }),
                io: Mutex::new(Some(io)),
                oneshot,
                readonly,
            }),
        })
    }

    /// Use this function to get the size of the data stream in an
    /// [`AsyncIo`]. Translation of `SDL_GetAsyncIOSize()`.
    ///
    /// This call is _not_ asynchronous; it assumes that obtaining this info
    /// is a non-blocking operation in most reasonable cases.
    pub fn size(&self) -> Result<i64> {
        match lock(&self.inner.io).as_mut() {
            Some(stream) => stream.size(),
            None => Err(Error::invalid_param("asyncio")), // (already closed)
        }
    }

    /// Translation of `RequestAsyncIO()`.
    fn request<U: Send + 'static>(
        &self,
        kind: AsyncIoTaskType,
        buffer: Vec<u8>,
        offset: u64,
        queue: &AsyncIoQueue<U>,
        userdata: U,
    ) -> Result<()> {
        let task = AsyncIoTask {
            asyncio: self.clone(),
            kind,
            offset,
            flush: false,
            requested_size: buffer.len() as u64,
            buffer,
            result: AsyncIoResult::Complete,
            result_size: 0,
            app_userdata: userdata,
            queue: queue.inner.clone(),
        };

        {
            let mut state = lock(&self.inner.state);
            if !matches!(state.closing, Closing::Open) {
                return Err(Error::new("SDL_AsyncIO is closing, can't start new tasks"));
            }
            state.tasks += 1;
            queue.inner.tasks_inflight.fetch_add(1, Ordering::AcqRel);
        }

        // generic_asyncio_io() -> generic_asyncioqueue_queue_task(), which
        // always succeeds.
        queue_async_io_task(Box::new(task));
        Ok(())
    }

    /// Start an async read. Translation of `SDL_ReadAsyncIO()`.
    ///
    /// This function reads up to `buffer.len()` bytes from `offset` into
    /// `buffer`. The buffer is handed back, filled, in the request's
    /// [`AsyncIoOutcome`].
    ///
    /// Note that reading past the end of the file is not an error (the
    /// outcome reports how many bytes were actually transferred), but
    /// reading from an offset past the end of the file just reports zero
    /// bytes transferred.
    ///
    /// An [`AsyncIoQueue`] must be specified. The newly-created task will be
    /// added to it when it completes its work.
    pub fn read<U: Send + 'static>(
        &self,
        buffer: Vec<u8>,
        offset: u64,
        queue: &AsyncIoQueue<U>,
        userdata: U,
    ) -> Result<()> {
        self.request(AsyncIoTaskType::Read, buffer, offset, queue, userdata)
    }

    /// Start an async write. Translation of `SDL_WriteAsyncIO()`.
    ///
    /// This function writes all of `buffer` to the file at `offset`. The
    /// buffer is handed back in the request's [`AsyncIoOutcome`].
    ///
    /// An [`AsyncIoQueue`] must be specified. The newly-created task will be
    /// added to it when it completes its work.
    pub fn write<U: Send + 'static>(
        &self,
        buffer: Vec<u8>,
        offset: u64,
        queue: &AsyncIoQueue<U>,
        userdata: U,
    ) -> Result<()> {
        self.request(AsyncIoTaskType::Write, buffer, offset, queue, userdata)
    }

    /// Close and free any allocated resources for an async I/O object.
    /// Translation of `SDL_CloseAsyncIO()`.
    ///
    /// Closing a file is _also_ an asynchronous task! If a write failure
    /// were to happen during the closing process, for example, the task
    /// results will report it as usual.
    ///
    /// Closing a file that has been written to does not guarantee the
    /// data has made it to physical media; it may remain in the operating
    /// system's file cache, for later writing to disk. This means that a
    /// successfully-closed file can be lost if the system crashes or loses
    /// power in this small window. To prevent this, call this function with
    /// `flush` set to true. This will make the operation take longer, and
    /// perhaps increase system load in general, but a successful result
    /// guarantees that the data has made it to physical storage. Don't use
    /// this for temporary files, caches, and unimportant data, and
    /// definitely use it for crucial irreplaceable files, like game saves.
    ///
    /// This function guarantees that the close will happen after any other
    /// pending tasks to `self`, so it's safe to open a file, start several
    /// operations, close the file immediately, then check for all results
    /// later. This function will not block until the tasks have completed.
    ///
    /// Once this function returns successfully, no new requests can be
    /// started on this stream (from any handle). If it fails, it may be
    /// tried again later.
    pub fn close<U: Send + 'static>(
        &self,
        flush: bool,
        queue: &AsyncIoQueue<U>,
        userdata: U,
    ) -> Result<()> {
        let mut state = lock(&self.inner.state);
        if !matches!(state.closing, Closing::Open) {
            return Err(Error::new("Already closing"));
        }

        let task = Box::new(AsyncIoTask {
            asyncio: self.clone(),
            kind: AsyncIoTaskType::Close,
            offset: 0,
            flush,
            buffer: Vec::new(),
            result: AsyncIoResult::Complete,
            requested_size: 0,
            result_size: 0,
            app_userdata: userdata,
            queue: queue.inner.clone(),
        });

        if state.tasks == 0 {
            // no tasks? Queue the close task now.
            state.closing = Closing::Queued;
            state.tasks += 1;
            queue.inner.tasks_inflight.fetch_add(1, Ordering::AcqRel);
            queue_async_io_task(task);
        } else {
            state.closing = Closing::Pending(task);
        }
        Ok(())
    }
}

/// Translation of `GetAsyncIOTaskOutcome()`. Returns `None` for the close
/// task of a [`load_file_async`] request, which is not reported to the app.
fn get_async_io_task_outcome<U>(task: AsyncIoTask<U>) -> Option<AsyncIoOutcome<U>> {
    let asyncio = task.asyncio.clone();
    let inner = &asyncio.inner;
    let is_close = task.kind == AsyncIoTaskType::Close;

    // Take the completed task out of the SDL_AsyncIO that created it.
    {
        let mut state = lock(&inner.state);
        state.tasks -= 1;
        // see if it's time to queue a pending close request (close requested and no other pending tasks)
        if !is_close && state.tasks == 0 && matches!(state.closing, Closing::Pending(_)) {
            if let Closing::Pending(closing) =
                std::mem::replace(&mut state.closing, Closing::Queued)
            {
                state.tasks += 1;
                closing.inflight().fetch_add(1, Ordering::AcqRel);
                // (the generic backend's queue_task cannot fail)
                queue_async_io_task(closing);
            }
        }
    }

    // was this the result of a closing task? Finally destroy the asyncio.
    // (Dropping the last handle and the stream does that here.)
    let retval = !(is_close && inner.oneshot); // don't send the close task results on to the app, just the read task for these.

    task.queue.tasks_inflight.fetch_sub(1, Ordering::AcqRel);

    retval.then(|| AsyncIoOutcome {
        asyncio: if inner.oneshot {
            None
        } else {
            Some(task.asyncio)
        },
        kind: task.kind,
        result: task.result,
        buffer: task.buffer,
        offset: task.offset,
        bytes_requested: task.requested_size,
        bytes_transferred: task.result_size,
        userdata: task.app_userdata,
    })
}

impl<U: Send + 'static> AsyncIoQueue<U> {
    /// Create a task queue for tracking multiple I/O operations.
    /// Translation of `SDL_CreateAsyncIOQueue()`.
    ///
    /// Async I/O operations are assigned to a queue when started. The queue
    /// can be checked for completed tasks thereafter.
    pub fn new() -> Result<AsyncIoQueue<U>> {
        // SDL_SYS_CreateAsyncIOQueue_Generic()
        prepare_threadpool()?;
        Ok(AsyncIoQueue {
            inner: Arc::new(QueueInner {
                completed: Mutex::new(Vec::new()),
                condition: Condvar::new(),
                tasks_inflight: AtomicI32::new(0),
            }),
        })
    }

    /// Query an async I/O task queue for completed tasks.
    /// Translation of `SDL_GetAsyncIOResult()`.
    ///
    /// If a task assigned to this queue has finished, this will return its
    /// outcome. If there are no finished tasks, it returns `None` right
    /// away.
    pub fn get_result(&self) -> Option<AsyncIoOutcome<U>> {
        // generic_asyncioqueue_get_results()
        let task = lock(&self.inner.completed).pop()?;
        get_async_io_task_outcome(task)
    }

    /// Block until an async I/O task queue has a completed task.
    /// Translation of `SDL_WaitAsyncIOResult()`.
    ///
    /// This function puts the calling thread to sleep until there a task
    /// assigned to the queue that has finished, or `timeout` passes (`None`
    /// waits forever).
    ///
    /// If a task assigned to the queue has finished, this will return its
    /// outcome. If no task finished in time, or [`signal`](Self::signal)
    /// woke the waiter, it returns `None`.
    ///
    /// Note that by the nature of various platforms, more than one waiting
    /// thread may wake to handle a single task, but only one will obtain
    /// it, so `timeout` is a _maximum_ wait time, and this function may
    /// return `None` sooner.
    pub fn wait_result(&self, timeout: Option<Duration>) -> Option<AsyncIoOutcome<U>> {
        let task = self.wait_task(timeout)?;
        get_async_io_task_outcome(task)
    }

    /// Translation of `generic_asyncioqueue_wait_results()`.
    fn wait_task(&self, timeout: Option<Duration>) -> Option<AsyncIoTask<U>> {
        let data = &self.inner;
        let mut completed = lock(&data.completed);
        if completed.is_empty() {
            completed = match timeout {
                None => data
                    .condition
                    .wait(completed)
                    .unwrap_or_else(|e| e.into_inner()),
                Some(t) => {
                    data.condition
                        .wait_timeout(completed, t)
                        .unwrap_or_else(|e| e.into_inner())
                        .0
                }
            };
        }
        completed.pop()
    }

    /// Wake up any threads that are blocking in
    /// [`wait_result`](Self::wait_result). Translation of `SDL_SignalAsyncIOQueue()`.
    ///
    /// This will unblock any threads that are sleeping in a call to
    /// `wait_result` for the specified queue, and cause them to return from
    /// that function.
    ///
    /// This can be useful when destroying a queue to make sure nothing is
    /// touching it indefinitely. In this case, once this call completes, the
    /// caller should take measures to make sure any previously-blocked
    /// threads have returned from their wait and will not touch the queue
    /// again (perhaps by setting a flag to tell the threads to terminate and
    /// then using [`Thread::wait`] to make sure they've done so).
    pub fn signal(&self) {
        let _completed = lock(&self.inner.completed);
        self.inner.condition.notify_all();
    }
}

impl<U> Drop for AsyncIoQueue<U> {
    /// Translation of `SDL_DestroyAsyncIOQueue()`: block until any pending
    /// tasks complete, throwing their outcomes away.
    fn drop(&mut self) {
        while self.inner.tasks_inflight.load(Ordering::Acquire) > 0 {
            let task = {
                let mut completed = lock(&self.inner.completed);
                if completed.is_empty() {
                    completed = self
                        .inner
                        .condition
                        .wait(completed)
                        .unwrap_or_else(|e| e.into_inner());
                }
                completed.pop()
            };
            if let Some(task) = task {
                // (the buffer of a load_file_async read is simply dropped)
                let _ = get_async_io_task_outcome(task); // this frees the task, and does other upkeep.
            }
        }
    }
}

/// Translation of `SDL_QuitAsyncIO()`.
pub(crate) fn quit_async_io() {
    shutdown_threadpool();
}

/// Load all the data from a file path, asynchronously.
/// Translation of `SDL_LoadFileAsync()`.
///
/// This function returns as quickly as possible; it does not wait for the
/// read to complete. On a successful return, this work will continue in the
/// background. If the work begins, even failure is asynchronous: a failing
/// return value from this function only means the work couldn't start at
/// all.
///
/// The data is read into a buffer the size of the file, handed back in the
/// [`AsyncIoOutcome`] of the read task (whose `asyncio` is `None`). Only
/// that read task is reported; the file is closed internally. (C SDL
/// over-allocates by one byte for a NUL terminator, which Rust does not
/// need.)
pub fn load_file_async<U: Clone + Send + 'static>(
    file: &str,
    queue: &AsyncIoQueue<U>,
    userdata: U,
) -> Result<()> {
    let asyncio = AsyncIo::open(file, "r", true)?; // (asyncio->oneshot = true)

    let retval = asyncio.size().and_then(|flen| {
        // !!! FIXME: check if flen > address space, since it'll truncate and we'll just end up with an incomplete buffer or a crash.
        let flen = usize::try_from(flen).map_err(|_| Error::out_of_memory())?;
        asyncio.read(vec![0u8; flen], 0, queue, userdata.clone())
    });

    let _ = asyncio.close(false, queue, userdata); // if this fails, we'll have a resource leak, but this would already be a dramatic system failure.

    retval
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TempDir, TEST_LOCK};

    /// `init::quit()` (called by other tests) shuts the threadpool down.
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait<U: Send + 'static>(queue: &AsyncIoQueue<U>) -> AsyncIoOutcome<U> {
        queue
            .wait_result(Some(Duration::from_secs(10)))
            .expect("a task should complete")
    }

    #[test]
    fn read_write_close() {
        let _l = guard();
        let tmp = TempDir::new("asyncio");
        let path = tmp.path("f.bin");
        let queue: AsyncIoQueue<&'static str> = AsyncIoQueue::new().unwrap();
        assert!(queue.get_result().is_none());

        assert_eq!(
            AsyncIo::from_file(&path, "a").unwrap_err().message(),
            "Unsupported file mode"
        );
        assert!(AsyncIo::from_file(&path, "r").is_err(), "must exist");

        let io = AsyncIo::from_file(&path, "w+").unwrap();
        io.write(b"hello world".to_vec(), 0, &queue, "w1").unwrap();
        let out = wait(&queue);
        assert_eq!(out.kind, AsyncIoTaskType::Write);
        assert_eq!(out.result, AsyncIoResult::Complete);
        assert_eq!(out.bytes_transferred, 11);
        assert_eq!(out.buffer, b"hello world");
        assert_eq!(out.userdata, "w1");
        assert_eq!(out.asyncio.as_ref(), Some(&io));
        assert_eq!(io.size().unwrap(), 11);

        // Read past the end: completes, short.
        io.read(vec![0; 8], 6, &queue, "r1").unwrap();
        let out = wait(&queue);
        assert_eq!(out.result, AsyncIoResult::Complete);
        assert_eq!(out.bytes_requested, 8);
        assert_eq!(out.data(), b"world");

        // Several requests, then close: the close comes last.
        for i in 0..4u64 {
            io.read(vec![0; 1], i, &queue, "multi").unwrap();
        }
        io.close(true, &queue, "close").unwrap();
        assert_eq!(
            io.close(false, &queue, "again").unwrap_err().message(),
            "Already closing"
        );
        assert_eq!(
            io.read(vec![0; 1], 0, &queue, "late")
                .unwrap_err()
                .message(),
            "SDL_AsyncIO is closing, can't start new tasks"
        );
        let mut got = Vec::new();
        for _ in 0..5 {
            let out = wait(&queue);
            got.push((out.userdata, out.offset, out.buffer));
        }
        assert_eq!(got.last().unwrap().0, "close", "{got:?}");
        got.pop();
        got.sort();
        assert_eq!(
            got,
            [
                ("multi", 0, b"h".to_vec()),
                ("multi", 1, b"e".to_vec()),
                ("multi", 2, b"l".to_vec()),
                ("multi", 3, b"l".to_vec())
            ]
        );
        assert!(queue.get_result().is_none());
        drop(io);
        assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
    }

    #[test]
    fn load_file_and_signal() {
        let _l = guard();
        let tmp = TempDir::new("asyncio-load");
        let path = tmp.path("data.txt");
        std::fs::write(&path, b"async contents").unwrap();
        let queue: AsyncIoQueue<u32> = AsyncIoQueue::new().unwrap();
        load_file_async(&path, &queue, 7).unwrap();
        let out = wait(&queue);
        assert_eq!(out.asyncio, None);
        assert_eq!(out.kind, AsyncIoTaskType::Read);
        assert_eq!(out.data(), b"async contents");
        assert_eq!(out.userdata, 7);
        // The internal close task is never reported.
        assert!(queue.wait_result(Some(Duration::from_millis(50))).is_none());
        assert!(load_file_async(&tmp.path("missing"), &queue, 0).is_err());

        // signal() wakes a waiter with nothing to report.
        let queue = Arc::new(AsyncIoQueue::<()>::new().unwrap());
        let q2 = queue.clone();
        let waiter = std::thread::spawn(move || q2.wait_result(None).is_none());
        while Arc::strong_count(&queue) > 1 && !waiter.is_finished() {
            queue.signal();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn dropping_queue_waits_for_tasks() {
        let _l = guard();
        let tmp = TempDir::new("asyncio-drop");
        let path = tmp.path("w.bin");
        let io = AsyncIo::from_file(&path, "w").unwrap();
        {
            let queue: AsyncIoQueue = AsyncIoQueue::new().unwrap();
            for i in 0..8u64 {
                io.write(vec![b'a' + i as u8; 4], i * 4, &queue, ())
                    .unwrap();
            }
            io.close(false, &queue, ()).unwrap();
        } // blocks until all nine tasks are done
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"aaaabbbbccccddddeeeeffffgggghhhh"
        );
    }
}
