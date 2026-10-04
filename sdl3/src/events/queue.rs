// Rust translation of src/events/SDL_events.c and SDL_eventwatch.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! General event handling code for SDL: the event queue, event watchers and
//! filters, enabled/disabled event types, user event registration, and the
//! main-thread callback queue.
//!
//! Upstream keeps `const char *` payloads alive through a thread-local
//! "temporary memory" list that is transferred into and out of queued
//! events. Rust events own their `String`s, so the whole
//! `SDL_TemporaryMemory` machinery (`SDL_AllocateTemporaryMemory`,
//! `SDL_ClaimTemporaryMemory`, `SDL_FreeTemporaryMemory`, ...) has no
//! translation: it is simply unnecessary.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::window::video;
use super::{Event, EventType};
use crate::error::Result;
use crate::hints;
use crate::init::InitFlags;
use crate::thread::{RawMutex, RawMutexGuard, Semaphore};
use crate::timer;
use crate::{err, init};

/// An arbitrary limit so we don't have unbounded growth.
/// Translation of `SDL_MAX_QUEUED_EVENTS`.
pub const MAX_QUEUED_EVENTS: usize = 65535;

/// Determines how often we pump events if joystick or sensor subsystems are active.
/// Translation of `ENUMERATION_POLL_INTERVAL_NS`.
const ENUMERATION_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Determines how often to pump events if joysticks or sensors are actively being read.
/// Translation of `EVENT_POLL_INTERVAL_NS`.
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(1);

// Determines how often to pump events if DBus is active
#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
const DBUS_POLL_INTERVAL: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// The event lock (SDL_event_lock)
// ---------------------------------------------------------------------------

/// The recursive lock protecting the event watch lists, the touch device list
/// and other event-system shared state. Translation of `SDL_event_lock`.
///
/// Upstream creates and destroys this mutex from `SDL_InitMainThread()`; a
/// `const`-constructible static needs neither step.
pub(crate) static EVENT_LOCK: RawMutex = RawMutex::new();

/// Translation of `SDL_CreateEventLock()`. The Rust lock is a static, so this
/// only exists to keep the init sequence recognizable.
pub(crate) fn create_event_lock() {}

/// Translation of `SDL_DestroyEventLock()`. See [`create_event_lock`].
pub(crate) fn destroy_event_lock() {}

/// Lock [`EVENT_LOCK`] for the current scope.
pub(crate) fn lock_events() -> RawMutexGuard<'static> {
    EVENT_LOCK.guard()
}

// ---------------------------------------------------------------------------
// Event watch lists (SDL_eventwatch.c)
// ---------------------------------------------------------------------------

/// A filter: returns `true` to keep the event. Translation of `SDL_EventFilter`
/// used as a filter; the `void *userdata` is captured by the closure.
pub(crate) type FilterFn = dyn Fn(&Event) -> bool + Send + Sync;
/// A watcher: observes the event. Translation of `SDL_EventFilter` used as a watcher.
pub(crate) type WatchFn = dyn Fn(&Event) + Send + Sync;

/// Translation of `SDL_EventWatcher` (watcher flavour).
struct Watcher {
    id: u64,
    callback: Arc<WatchFn>,
    removed: Arc<AtomicBool>,
}

#[derive(Default)]
struct WatchListInner {
    filter: Option<Arc<FilterFn>>,
    watchers: Vec<Watcher>,
    dispatching: bool,
    removed: bool,
}

/// A filter plus a list of watchers. Translation of `SDL_EventWatchList`.
///
/// All operations take [`EVENT_LOCK`] (recursively, like upstream), so a
/// watcher may add or remove watchers from inside its callback. The inner
/// `Mutex` is only held for bookkeeping, never across a callback.
pub(crate) struct WatchList {
    inner: Mutex<WatchListInner>,
}

static NEXT_WATCHER_ID: AtomicU64 = AtomicU64::new(1);

impl WatchList {
    /// Translation of `SDL_InitEventWatchList()` (an empty list).
    pub(crate) const fn new() -> Self {
        WatchList {
            inner: Mutex::new(WatchListInner {
                filter: None,
                watchers: Vec::new(),
                dispatching: false,
                removed: false,
            }),
        }
    }

    fn inner(&self) -> MutexGuard<'_, WatchListInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Translation of `SDL_QuitEventWatchList()`.
    pub(crate) fn quit(&self) {
        let mut l = self.inner();
        l.watchers.clear();
        l.filter = None;
        l.dispatching = false;
        l.removed = false;
    }

    /// Run the filter (if any) and then every watcher. Returns `false` if the
    /// filter rejected the event. Translation of `SDL_DispatchEventWatchList()`.
    pub(crate) fn dispatch(&self, event: &Event) -> bool {
        // Make sure we only dispatch the current watcher list
        let (filter, snapshot) = {
            let l = self.inner();
            if l.filter.is_none() && l.watchers.is_empty() {
                return true;
            }
            let snapshot: Vec<(Arc<WatchFn>, Arc<AtomicBool>)> = l
                .watchers
                .iter()
                .map(|w| (w.callback.clone(), w.removed.clone()))
                .collect();
            (l.filter.clone(), snapshot)
        };

        let _lock = lock_events();

        if let Some(filter) = filter {
            if !filter(event) {
                return false;
            }
        }

        self.inner().dispatching = true;
        for (callback, removed) in &snapshot {
            if !removed.load(Ordering::Acquire) {
                callback(event);
            }
        }

        let mut l = self.inner();
        l.dispatching = false;
        if l.removed {
            l.watchers.retain(|w| !w.removed.load(Ordering::Acquire));
            l.removed = false;
        }
        true
    }

    /// Translation of `SDL_AddEventWatchList()`; returns the watcher's id.
    pub(crate) fn add(&self, callback: Arc<WatchFn>) -> u64 {
        let _lock = lock_events();
        let id = NEXT_WATCHER_ID.fetch_add(1, Ordering::Relaxed);
        self.inner().watchers.push(Watcher {
            id,
            callback,
            removed: Arc::new(AtomicBool::new(false)),
        });
        id
    }

    /// Translation of `SDL_RemoveEventWatchList()`.
    pub(crate) fn remove(&self, id: u64) {
        let _lock = lock_events();
        let mut l = self.inner();
        if let Some(i) = l.watchers.iter().position(|w| w.id == id) {
            if l.dispatching {
                l.watchers[i].removed.store(true, Ordering::Release);
                l.removed = true;
            } else {
                l.watchers.remove(i);
            }
        }
    }

    fn set_filter(&self, filter: Option<Arc<FilterFn>>) {
        self.inner().filter = filter;
    }

    fn filter(&self) -> Option<Arc<FilterFn>> {
        self.inner().filter.clone()
    }
}

/// A registered event watcher; dropping it removes the watcher.
///
/// Translation of the (`SDL_EventFilter`, `void *userdata`) pair that
/// identifies a watcher to `SDL_RemoveEventWatch()`. Use
/// [`EventWatch::detach`] to keep watching for the life of the process.
#[must_use = "dropping an EventWatch unregisters it; call .detach() to keep it"]
pub struct EventWatch {
    list: &'static WatchList,
    id: u64,
}

impl std::fmt::Debug for EventWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventWatch").field("id", &self.id).finish()
    }
}

impl EventWatch {
    pub(crate) fn new(list: &'static WatchList, id: u64) -> Self {
        EventWatch { list, id }
    }

    /// Stop watching. Translation of `SDL_RemoveEventWatch()`.
    pub fn remove(self) {
        drop(self);
    }

    /// Keep watching for the life of the process.
    pub fn detach(self) {
        std::mem::forget(self);
    }
}

impl Drop for EventWatch {
    fn drop(&mut self) {
        self.list.remove(self.id);
    }
}

// ---------------------------------------------------------------------------
// Event queue state
// ---------------------------------------------------------------------------

/// Translation of `SDL_event_watchers`.
static EVENT_WATCHERS: WatchList = WatchList::new();
/// Translation of `SDL_sentinel_pending`.
static SENTINEL_PENDING: AtomicI32 = AtomicI32::new(0);
/// Translation of `SDL_last_event_id`.
static LAST_EVENT_ID: AtomicU32 = AtomicU32::new(0);
/// Translation of `SDL_userevents`.
static USER_EVENTS: AtomicI32 = AtomicI32::new(0);

/// Translation of `SDL_disabled_events[256]` (blocks of `Uint32 bits[8]`),
/// flattened into one lock-free bitset indexed by the low 16 bits of the type.
static DISABLED_EVENTS: [AtomicU32; 256 * 8] = {
    #[allow(clippy::declare_interior_mutable_const)] // array-repeat initializer only
    const ZERO: AtomicU32 = AtomicU32::new(0);
    [ZERO; 256 * 8]
};

/// Translation of the `SDL_EventQ` struct (minus the free list, which `Vec`
/// handles).
struct EventQueue {
    active: bool,
    max_events_seen: usize,
    events: VecDeque<Event>,
}

static EVENT_Q: Mutex<EventQueue> = Mutex::new(EventQueue {
    active: false,
    max_events_seen: 0,
    events: VecDeque::new(),
});
/// Translation of `SDL_EventQ.count` (readable without the lock).
static EVENT_Q_COUNT: AtomicI32 = AtomicI32::new(0);

fn queue() -> MutexGuard<'static, EventQueue> {
    EVENT_Q.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// Hints
// ---------------------------------------------------------------------------

/// Translation of `SDL_update_joysticks`.
static UPDATE_JOYSTICKS: AtomicBool = AtomicBool::new(true);
/// Translation of `SDL_update_sensors`.
static UPDATE_SENSORS: AtomicBool = AtomicBool::new(true);

/// Verbosity of logged events as defined in `SDL_HINT_EVENT_LOGGING`:
///  - 0: (default) no logging
///  - 1: logging of most events
///  - 2: as above, plus mouse, pen, and finger motion
///
/// Translation of `SDL_EventLoggingVerbosity`.
static EVENT_LOGGING_VERBOSITY: AtomicI32 = AtomicI32::new(0);

/// Hint watchers installed by [`init_events`]; dropping them unregisters.
static HINT_CALLBACKS: Mutex<Vec<hints::Callback>> = Mutex::new(Vec::new());

/// Translation of `SDL_AutoUpdateJoysticksChanged()`.
fn auto_update_joysticks_changed(hint: Option<&str>) {
    UPDATE_JOYSTICKS.store(hints::string_to_bool(hint, true), Ordering::Relaxed);
}

/// Translation of `SDL_AutoUpdateSensorsChanged()`.
fn auto_update_sensors_changed(hint: Option<&str>) {
    UPDATE_SENSORS.store(hints::string_to_bool(hint, true), Ordering::Relaxed);
}

/// Translation of `SDL_PollSentinelChanged()`.
fn poll_sentinel_changed(hint: Option<&str>) {
    set_event_enabled(EventType::POLL_SENTINEL, hints::string_to_bool(hint, true));
}

/// Translation of `SDL_EventLoggingChanged()`.
fn event_logging_changed(hint: Option<&str>) {
    let v = match hint {
        Some(h) if !h.is_empty() => crate::stdlib::atoi(h).clamp(0, 3),
        _ => 0,
    };
    EVENT_LOGGING_VERBOSITY.store(v, Ordering::Relaxed);
}

/// The current `SDL_HINT_EVENT_LOGGING` verbosity (0–3).
pub(crate) fn event_logging_verbosity() -> i32 {
    EVENT_LOGGING_VERBOSITY.load(Ordering::Relaxed)
}

/// Translation of `SDL_LogEvent()`.
fn log_event(event: &Event) {
    // sensor/mouse/pen/finger/pinch motion are spammy, ignore these if they aren't demanded.
    let t = event.event_type();
    if event_logging_verbosity() < 2
        && matches!(
            t,
            EventType::MOUSE_MOTION
                | EventType::FINGER_MOTION
                | EventType::PEN_AXIS
                | EventType::PEN_MOTION
                | EventType::PINCH_UPDATE
                | EventType::GAMEPAD_AXIS_MOTION
                | EventType::GAMEPAD_SENSOR_UPDATE
                | EventType::GAMEPAD_TOUCHPAD_MOTION
                | EventType::GAMEPAD_UPDATE_COMPLETE
                | EventType::JOYSTICK_AXIS_MOTION
                | EventType::JOYSTICK_UPDATE_COMPLETE
                | EventType::SENSOR_UPDATE
        )
    {
        return;
    }

    let buf = event.description();
    if !buf.is_empty() {
        crate::log::info!("SDL EVENT: {buf}");
    }
}

// ---------------------------------------------------------------------------
// Event loop start/stop
// ---------------------------------------------------------------------------

/// Translation of `SDL_StopEventLoop()`.
fn stop_event_loop() {
    let report = hints::get("SDL_EVENT_QUEUE_STATISTICS");

    let mut q = queue();

    q.active = false;

    if report
        .as_deref()
        .is_some_and(|r| crate::stdlib::atoi(r) != 0)
    {
        crate::log::info!(
            "SDL EVENT QUEUE: Maximum events in-flight: {}",
            q.max_events_seen
        );
    }

    // Clean out EventQ
    q.events.clear();
    EVENT_Q_COUNT.store(0, Ordering::Relaxed);
    q.max_events_seen = 0;
    SENTINEL_PENDING.store(0, Ordering::Relaxed);

    // Clear disabled event state
    for word in DISABLED_EVENTS.iter() {
        word.store(0, Ordering::Relaxed);
    }

    EVENT_WATCHERS.quit();
    super::window::quit_window_event_watch();
}

/// This function (and associated calls) may be called more than once.
/// Translation of `SDL_StartEventLoop()`.
fn start_event_loop() -> Result<()> {
    /* We'll leave the event queue alone, since we might have gotten
      some important events at launch (like SDL_EVENT_DROP_FILE)

      FIXME: Does this introduce any other bugs with events at startup?
    */

    // Create the lock and set ourselves active
    let mut q = queue();

    super::window::init_window_event_watch();

    q.active = true;
    Ok(())
}

// ---------------------------------------------------------------------------
// Queue primitives
// ---------------------------------------------------------------------------

/// Add an event to the event queue -- called with the queue locked.
/// Translation of `SDL_AddEvent()`; returns the number of events added (0 or 1).
fn add_event(q: &mut EventQueue, event: Event) -> Result<usize> {
    let initial_count = EVENT_Q_COUNT.load(Ordering::Relaxed);

    if initial_count as usize >= MAX_QUEUED_EVENTS {
        return Err(err!("Event queue is full ({initial_count} events)"));
    }

    if event_logging_verbosity() > 0 {
        log_event(&event);
    }

    if event.event_type() == EventType::POLL_SENTINEL {
        SENTINEL_PENDING.fetch_add(1, Ordering::Relaxed);
    }

    q.events.push_back(event);

    let final_count = (EVENT_Q_COUNT.fetch_add(1, Ordering::Relaxed) + 1) as usize;
    if final_count > q.max_events_seen {
        q.max_events_seen = final_count;
    }

    LAST_EVENT_ID.fetch_add(1, Ordering::Relaxed);

    Ok(1)
}

/// Remove an event from the queue -- called with the queue locked.
/// Translation of `SDL_CutEvent()`.
fn cut_event(q: &mut EventQueue, index: usize) -> Event {
    let event = q.events.remove(index).expect("cut_event index in range");

    if event.event_type() == EventType::POLL_SENTINEL {
        SENTINEL_PENDING.fetch_sub(1, Ordering::Relaxed);
    }

    debug_assert!(EVENT_Q_COUNT.load(Ordering::Relaxed) > 0);
    EVENT_Q_COUNT.fetch_sub(1, Ordering::Relaxed);
    event
}

/// Translation of `SDL_SendWakeupEvent()`.
fn send_wakeup_event() {
    if let Some(video) = video() {
        video.send_wakeup_event();
    }
}

/// The action for [`peep_events_internal`]. Translation of `SDL_EventAction`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PeepAction {
    /// Check but don't remove events from the queue front.
    Peek,
    /// Retrieve/remove events from the front of the queue.
    Get,
}

/// Lock the event queue, take a peep at it, and unlock it.
/// Translation of `SDL_PeepEventsInternal()` for the `PEEK`/`GET` actions;
/// `limit == None` means "all". Returns `Err` after the event system shut down.
fn peep_events_internal(
    limit: Option<usize>,
    action: PeepAction,
    min_type: EventType,
    max_type: EventType,
    include_sentinel: bool,
) -> Result<Vec<Event>> {
    let mut used = Vec::new();
    let mut sentinels_expected = 0;

    // Lock the event queue
    let mut q = queue();

    // Don't look after we've quit
    if !q.active {
        // We get a few spurious events at shutdown, so don't warn then
        return Err(err!("The event system has been shut down"));
    }

    let mut i = 0;
    while i < q.events.len() && limit.is_none_or(|n| used.len() < n) {
        let ty = q.events[i].event_type();
        let mut next = i + 1;
        if min_type <= ty && ty <= max_type {
            let event = if action == PeepAction::Get {
                next = i;
                cut_event(&mut q, i)
            } else {
                q.events[i].clone()
            };
            if ty == EventType::POLL_SENTINEL {
                // Special handling for the sentinel event
                if !include_sentinel {
                    // Skip it, we don't want to include it
                    i = next;
                    continue;
                }
                if action != PeepAction::Get {
                    sentinels_expected += 1;
                }
                if SENTINEL_PENDING.load(Ordering::Relaxed) > sentinels_expected {
                    // Skip it, there's another one pending
                    i = next;
                    continue;
                }
            }
            used.push(event);
        }
        i = next;
    }
    drop(q);

    Ok(used)
}

/// Add events to the back of the queue. Translation of `SDL_PeepEvents()`
/// with `SDL_ADDEVENT`; returns the number of events actually added.
///
/// Events are added one at a time; the first failure (queue full, or the
/// event system shut down) stops the loop and is reported, after the ones
/// before it have been queued.
pub fn add_events(events: impl IntoIterator<Item = Event>) -> Result<usize> {
    let mut used = 0;
    let result = {
        let mut q = queue();
        // Don't look after we've quit
        if !q.active {
            Err(err!("The event system has been shut down"))
        } else {
            let mut result = Ok(());
            for event in events {
                match add_event(&mut q, event) {
                    Ok(n) => used += n,
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            result
        }
    };

    if used > 0 {
        send_wakeup_event();
    }

    result.map(|()| used)
}

/// Copy up to `limit` events of types in `min_type..=max_type` from the front
/// of the queue without removing them. Translation of `SDL_PeepEvents()` with
/// `SDL_PEEKEVENT`. Returns `Err` once the event system has shut down.
pub fn peek_events(min_type: EventType, max_type: EventType, limit: usize) -> Result<Vec<Event>> {
    peep_events_internal(Some(limit), PeepAction::Peek, min_type, max_type, false)
}

/// Remove and return up to `limit` events of types in `min_type..=max_type`
/// from the front of the queue. Translation of `SDL_PeepEvents()` with
/// `SDL_GETEVENT`.
pub fn get_events(min_type: EventType, max_type: EventType, limit: usize) -> Result<Vec<Event>> {
    peep_events_internal(Some(limit), PeepAction::Get, min_type, max_type, false)
}

/// Check for the existence of a certain event type in the event queue.
/// Translation of `SDL_HasEvent()`.
pub fn has_event(event_type: EventType) -> bool {
    has_events(event_type, event_type)
}

/// Check for the existence of certain event types in the event queue.
/// Translation of `SDL_HasEvents()`.
pub fn has_events(min_type: EventType, max_type: EventType) -> bool {
    let q = queue();
    q.active
        && q.events.iter().any(|e| {
            let t = e.event_type();
            min_type <= t && t <= max_type
        })
}

/// Clear events of a specific type from the event queue.
/// Translation of `SDL_FlushEvent()`.
pub fn flush_event(event_type: EventType) {
    flush_events(event_type, event_type)
}

/// Clear events of a range of types from the event queue.
/// Translation of `SDL_FlushEvents()`.
pub fn flush_events(min_type: EventType, max_type: EventType) {
    // Make sure the events are current
    /* Actually, we can't do this since we might be flushing while processing
       a resize event, and calling this might trigger further resize events.
    */
    // (upstream: `#if 0 SDL_PumpEvents(); #endif`)

    // Lock the event queue
    let mut q = queue();
    // Don't look after we've quit
    if !q.active {
        return;
    }
    let mut i = 0;
    while i < q.events.len() {
        let t = q.events[i].event_type();
        if min_type <= t && t <= max_type {
            cut_event(&mut q, i);
        } else {
            i += 1;
        }
    }
}

/// Remove every queued event the filter rejects. The filter runs without any
/// queue lock held (upstream calls it under the recursive queue lock), so it
/// may call back into this module.
fn cut_rejected_events(filter: &dyn Fn(&Event) -> bool) {
    let drained: Vec<Event> = {
        let mut q = queue();
        let drained: Vec<Event> = q.events.drain(..).collect();
        EVENT_Q_COUNT.fetch_sub(drained.len() as i32, Ordering::Relaxed);
        let sentinels = drained
            .iter()
            .filter(|e| e.event_type() == EventType::POLL_SENTINEL)
            .count() as i32;
        SENTINEL_PENDING.fetch_sub(sentinels, Ordering::Relaxed);
        drained
    };

    let kept: Vec<Event> = drained.into_iter().filter(|e| filter(e)).collect();

    let mut q = queue();
    let sentinels = kept
        .iter()
        .filter(|e| e.event_type() == EventType::POLL_SENTINEL)
        .count() as i32;
    SENTINEL_PENDING.fetch_add(sentinels, Ordering::Relaxed);
    EVENT_Q_COUNT.fetch_add(kept.len() as i32, Ordering::Relaxed);
    // Anything pushed while we were filtering stays after the kept events,
    // exactly where it would have landed in the in-place walk.
    for (i, e) in kept.into_iter().enumerate() {
        q.events.insert(i, e);
    }
}

// ---------------------------------------------------------------------------
// Main thread callbacks
// ---------------------------------------------------------------------------

/// Translation of `SDL_MainThreadCallbackState`.
const MAIN_CALLBACK_WAITING: i32 = 0;
const MAIN_CALLBACK_COMPLETE: i32 = 1;
const MAIN_CALLBACK_CANCELED: i32 = 2;

/// Translation of `SDL_MainThreadCallbackEntry`.
struct MainThreadCallbackEntry {
    callback: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    state: AtomicI32,
    semaphore: Option<Semaphore>,
}

/// Translation of `SDL_main_callbacks_lock/head/tail`.
static MAIN_CALLBACKS: Mutex<Vec<Arc<MainThreadCallbackEntry>>> = Mutex::new(Vec::new());

fn main_callbacks() -> MutexGuard<'static, Vec<Arc<MainThreadCallbackEntry>>> {
    MAIN_CALLBACKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_CreateMainThreadCallback()`.
fn create_main_thread_callback(
    callback: Box<dyn FnOnce() + Send>,
    wait_complete: bool,
) -> Arc<MainThreadCallbackEntry> {
    Arc::new(MainThreadCallbackEntry {
        callback: Mutex::new(Some(callback)),
        state: AtomicI32::new(MAIN_CALLBACK_WAITING),
        semaphore: wait_complete.then(|| Semaphore::new(0)),
    })
}

/// Translation of `SDL_InitMainThreadCallbacks()`.
fn init_main_thread_callbacks() {
    debug_assert!(main_callbacks().is_empty());
}

/// Translation of `SDL_QuitMainThreadCallbacks()`.
fn quit_main_thread_callbacks() {
    let entries = std::mem::take(&mut *main_callbacks());

    for entry in entries {
        if let Some(sem) = &entry.semaphore {
            // Let the waiting thread know this is canceled
            entry.state.store(MAIN_CALLBACK_CANCELED, Ordering::Release);
            sem.signal();
        }
        // Nobody's waiting for this, clean it up (Arc drop)
    }
}

/// Run every queued main-thread callback. Translation of `SDL_RunMainThreadCallbacks()`.
pub(crate) fn run_main_thread_callbacks() {
    let entries = std::mem::take(&mut *main_callbacks());

    for entry in entries {
        let callback = entry
            .callback
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(callback) = callback {
            callback();
        }

        if let Some(sem) = &entry.semaphore {
            // Let the waiting thread know this is done
            entry.state.store(MAIN_CALLBACK_COMPLETE, Ordering::Release);
            sem.signal();
        }
        // Nobody's waiting for this, clean it up (Arc drop)
    }
}

/// Run `callback` on the main thread during event processing.
///
/// If this is the main thread, or the events subsystem isn't initialized, the
/// callback runs immediately. Otherwise it is queued and runs the next time
/// the main thread pumps events; with `wait_complete` this blocks until then.
/// Translation of `SDL_RunOnMainThread()`.
pub fn run_on_main_thread(
    callback: impl FnOnce() + Send + 'static,
    wait_complete: bool,
) -> Result<()> {
    if init::is_main_thread() || init::was_init(init::InitFlags::EVENTS).is_empty() {
        // No need to queue the callback
        callback();
        return Ok(());
    }

    let entry = create_main_thread_callback(Box::new(callback), wait_complete);

    main_callbacks().push(entry.clone());

    // If the main thread is waiting for events, wake it up
    send_wakeup_event();

    if !wait_complete {
        // Queued for execution, wait not requested
        return Ok(());
    }

    entry
        .semaphore
        .as_ref()
        .expect("wait_complete entries have a semaphore")
        .wait();

    match entry.state.load(Ordering::Acquire) {
        // Execution complete!
        MAIN_CALLBACK_COMPLETE => Ok(()),
        // The callback was canceled on the main thread
        MAIN_CALLBACK_CANCELED => Err(err!("Callback canceled")),
        // Probably hit a deadlock in the callback
        _ => Err(err!("Callback timed out")),
    }
}

// ---------------------------------------------------------------------------
// Pumping
// ---------------------------------------------------------------------------

/// Periodic work done during every pump. Translation of `SDL_PumpEventMaintenance()`.
pub(crate) fn pump_event_maintenance() {
    #[cfg(target_os = "linux")]
    crate::core::linux::udev::poll();

    crate::audio::update_audio();

    crate::camera::update_camera();

    // Check for sensor state change
    if UPDATE_SENSORS.load(Ordering::Relaxed) {
        crate::sensor::update_sensors();
    }

    // Check for joystick state change
    if UPDATE_JOYSTICKS.load(Ordering::Relaxed) {
        crate::joystick::update_joysticks();
    }

    super::pen::send_pending_pen_proximity();

    super::mouse::update_cursor_animation();

    crate::tray::update_trays();

    super::quit::send_pending_signal_events();
}

/// Run the system dependent event loops. Translation of `SDL_PumpEventsInternal()`.
fn pump_events_internal(push_sentinel: bool) {
    // This should only be called on the main thread, check in debug builds
    debug_assert!(init::is_main_thread());

    // Release any keys held down from last frame
    super::keyboard::release_auto_release_keys();

    // Run any pending main thread callbacks
    run_main_thread_callbacks();

    // DBus event processing is independent of the video subsystem
    #[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
    crate::core::linux::dbus::pump_events();

    // Get events from the video subsystem
    if let Some(video) = video() {
        video.pump_events();
    }

    pump_event_maintenance();

    if push_sentinel && event_enabled(EventType::POLL_SENTINEL) {
        // Make sure we don't already have a sentinel in the queue, and add one to the end
        if SENTINEL_PENDING.load(Ordering::Relaxed) > 0 {
            let _ = peep_events_internal(
                Some(1),
                PeepAction::Get,
                EventType::POLL_SENTINEL,
                EventType::POLL_SENTINEL,
                true,
            );
        }

        let _ = push(Event::simple(EventType::POLL_SENTINEL, Duration::ZERO));
    }
}

/// Pump the event loop, gathering events from the input devices.
/// Translation of `SDL_PumpEvents()`.
pub fn pump() {
    pump_events_internal(false);
}

// Public functions

/// Poll for currently pending events. Translation of `SDL_PollEvent()`.
pub fn poll() -> Option<Event> {
    wait_timeout(Some(Duration::ZERO)).ok().flatten()
}

/// Translation of `SDL_events_get_polling_interval()`; `None` means no
/// periodic polling is required (`SDL_MAX_SINT64`).
fn events_get_polling_interval() -> Option<Duration> {
    let mut poll_interval: Option<Duration> = None;
    let mut min = |interval: Duration| {
        poll_interval = Some(poll_interval.map_or(interval, |p| p.min(interval)));
    };

    if !init::was_init(InitFlags::JOYSTICK).is_empty() && UPDATE_JOYSTICKS.load(Ordering::Relaxed) {
        if crate::joystick::joysticks_opened() {
            // If we have joysticks open, we need to poll rapidly for events
            min(EVENT_POLL_INTERVAL);
        } else {
            // If not, just poll every few seconds to enumerate new joysticks
            min(ENUMERATION_POLL_INTERVAL);
        }
    }

    if !init::was_init(InitFlags::SENSOR).is_empty()
        && UPDATE_SENSORS.load(Ordering::Relaxed)
        && crate::sensor::sensors_opened()
    {
        // If we have sensors open, we need to poll rapidly for events
        min(EVENT_POLL_INTERVAL);
    }

    // (Tray polling arrives with the D-Bus tray.)

    // Wake periodically to pump DBus events
    #[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
    min(DBUS_POLL_INTERVAL);

    poll_interval
}

/// Translation of `SDL_WaitEventTimeout_Device()`. Returns `Ok(Some)` with an
/// event, `Ok(None)` on timeout and `Err` if the backend can't reliably wait.
fn wait_event_timeout_device(
    video: &dyn super::window::VideoHooks,
    wakeup_window: super::WindowID,
    start: Duration,
    timeout: Option<Duration>,
) -> std::result::Result<Option<Event>, ()> {
    let mut loop_timeout = timeout;
    let poll_interval = events_get_polling_interval();

    loop {
        /* Pump events on entry and each time we wake to ensure:
           a) All pending events are batch processed after waking up from a wait
           b) Waiting can be completely skipped if events are already available to be pumped
           c) Periodic processing that takes place in some platform PumpEvents() functions happens
           d) Signals received in WaitEventTimeout() are turned into SDL events
        */
        pump_events_internal(true);

        match get_events(EventType::FIRST, EventType::LAST, 1) {
            // Got an error: return
            Err(_) => return Ok(None),
            Ok(mut events) if !events.is_empty() => {
                // There is an event, we can return.
                return Ok(Some(events.remove(0)));
            }
            Ok(_) => {}
        }
        // No events found in the queue, call WaitEventTimeout to wait for an event.
        if let Some(timeout) = timeout {
            let elapsed = timer::ticks().saturating_sub(start);
            if elapsed >= timeout {
                return Ok(None);
            }
            loop_timeout = Some(timeout - elapsed);
        }
        // Adjust the timeout for any polling requirements we currently have.
        if let Some(poll_interval) = poll_interval {
            loop_timeout = Some(match loop_timeout {
                Some(t) => t.min(poll_interval),
                None => poll_interval,
            });
        }
        video.set_wakeup_window(Some(wakeup_window));
        let status = video.wait_event_timeout(loop_timeout);
        video.set_wakeup_window(None);
        if status == 0 && poll_interval.is_some() && loop_timeout == poll_interval {
            // We may have woken up to poll. Try again
            continue;
        } else if status == 0 {
            // The timeout is elapsed: return
            return Ok(None);
        } else if status < 0 {
            // There is an error: return
            return Err(());
        }
        /* An event was found and pumped into the SDL events queue. Continue the loop
        to let SDL_PeepEvents pick it up .*/
    }
}

/// Wait indefinitely for the next available event. Translation of `SDL_WaitEvent()`.
pub fn wait() -> Result<Event> {
    match wait_timeout(None)? {
        Some(event) => Ok(event),
        None => Err(err!("The event system has been shut down")),
    }
}

/// Wait until the specified timeout for the next available event.
///
/// `None` waits forever; `Some(Duration::ZERO)` polls. Returns `Ok(None)` when
/// the timeout elapsed (or, when polling, when the poll cycle ended) without an
/// event, and `Err` if the event system has been shut down.
/// Translation of `SDL_WaitEventTimeoutNS()`.
pub fn wait_timeout(timeout: Option<Duration>) -> Result<Option<Event>> {
    let include_sentinel = timeout == Some(Duration::ZERO);

    let (start, expiration) = match timeout {
        Some(t) if t > Duration::ZERO => {
            let start = timer::ticks();
            (start, start + t)
        }
        _ => (Duration::ZERO, Duration::ZERO),
    };

    // If there isn't a poll sentinel event pending, pump events and add one
    if SENTINEL_PENDING.load(Ordering::Relaxed) == 0 {
        pump_events_internal(true);
    }

    // First check for existing events
    let mut result = peep_events_internal(
        Some(1),
        PeepAction::Get,
        EventType::FIRST,
        EventType::LAST,
        include_sentinel,
    )?;
    if include_sentinel {
        if let Some(event) = result.first() {
            if event.event_type() == EventType::POLL_SENTINEL {
                // Reached the end of a poll cycle, and not willing to wait
                return Ok(None);
            }
        }
    }
    if result.is_empty() {
        if timeout == Some(Duration::ZERO) {
            // No events available, and not willing to wait
            return Ok(None);
        }
    } else {
        // Has existing events
        return Ok(Some(result.remove(0)));
    }
    // We should have completely handled timeoutNS == 0 above
    debug_assert!(timeout != Some(Duration::ZERO));

    if let Some(video) = video() {
        if video.can_wait() {
            // Look if a shown window is available to send the wakeup event.
            if let Some(wakeup_window) = video.find_active_window() {
                match wait_event_timeout_device(&*video, wakeup_window, start, timeout) {
                    Ok(result) => return Ok(result),
                    Err(()) => {
                        /* There may be implementation-defined conditions where the backend cannot
                         * reliably wait for the next event. If that happens, fall back to polling.
                         */
                    }
                }
            }
        }
    }

    loop {
        pump_events_internal(true);

        let mut events = get_events(EventType::FIRST, EventType::LAST, 1)?;
        if !events.is_empty() {
            return Ok(Some(events.remove(0)));
        }

        let mut delay = EVENT_POLL_INTERVAL;
        if timeout.is_some_and(|t| t > Duration::ZERO) {
            let now = timer::ticks();
            if now >= expiration {
                // Timeout expired and no events
                return Ok(None);
            }
            delay = (expiration - now).min(delay);
        }
        timer::delay(delay);
    }
}

// ---------------------------------------------------------------------------
// Pushing, filtering, watching
// ---------------------------------------------------------------------------

/// Translation of `SDL_CallEventWatchers()`.
fn call_event_watchers(event: &Event) -> bool {
    if event.event_type() == EventType::POLL_SENTINEL {
        return true;
    }

    EVENT_WATCHERS.dispatch(event)
}

/// Add an event to the event queue.
///
/// The event's timestamp is filled in if it is zero. Event watchers run first
/// (on this thread); if the event filter rejects the event, `Ok(false)` is
/// returned and nothing is queued. `Err` means the queue is full or the event
/// system has shut down. Translation of `SDL_PushEvent()`.
pub fn push(mut event: Event) -> Result<bool> {
    if event.timestamp() == Duration::ZERO {
        *event.timestamp_mut() = timer::ticks();
    }

    if !call_event_watchers(&event) {
        return Ok(false);
    }

    add_events(std::iter::once(event)).map(|_| true)
}

/// Set up a filter to process all events before they are posted to the queue.
///
/// The filter is also run right now over the queued events, discarding any it
/// rejects. Translation of `SDL_SetEventFilter()`.
pub fn set_filter(filter: impl Fn(&Event) -> bool + Send + Sync + 'static) {
    let filter: Arc<FilterFn> = Arc::new(filter);
    let _lock = lock_events();
    // Set filter and discard pending events
    EVENT_WATCHERS.set_filter(Some(filter.clone()));
    // Cut all events not accepted by the filter
    cut_rejected_events(&*filter);
}

/// Remove the event filter. Translation of `SDL_SetEventFilter(NULL, NULL)`.
pub fn clear_filter() {
    let _lock = lock_events();
    EVENT_WATCHERS.set_filter(None);
}

/// Whether an event filter is set. Translation of `SDL_GetEventFilter()`.
pub fn has_filter() -> bool {
    let _lock = lock_events();
    EVENT_WATCHERS.filter().is_some()
}

/// Add a callback to be triggered when an event is added to the event queue.
///
/// The watcher runs on the thread that pushes the event, for every event
/// (except the internal poll sentinel), before the filter can drop it.
/// Translation of `SDL_AddEventWatch()`.
pub fn add_watch(callback: impl Fn(&Event) + Send + Sync + 'static) -> EventWatch {
    let id = EVENT_WATCHERS.add(Arc::new(callback));
    EventWatch::new(&EVENT_WATCHERS, id)
}

/// Run a specific filter function on the current event queue, removing any
/// events for which the filter returns `false`. Translation of `SDL_FilterEvents()`.
pub fn filter_events(filter: impl Fn(&Event) -> bool) {
    cut_rejected_events(&filter);
}

/// Set the state of processing events by type. Translation of `SDL_SetEventEnabled()`.
pub fn set_event_enabled(event_type: EventType, enabled: bool) {
    let (word, bit) = disabled_bit(event_type);
    let current_state = (DISABLED_EVENTS[word].load(Ordering::Relaxed) & bit) == 0;

    if enabled != current_state {
        if enabled {
            DISABLED_EVENTS[word].fetch_and(!bit, Ordering::Relaxed);

            // Gamepad events depend on joystick events
            match event_type {
                EventType::GAMEPAD_ADDED => set_event_enabled(EventType::JOYSTICK_ADDED, true),
                EventType::GAMEPAD_REMOVED => set_event_enabled(EventType::JOYSTICK_REMOVED, true),
                EventType::GAMEPAD_AXIS_MOTION
                | EventType::GAMEPAD_BUTTON_DOWN
                | EventType::GAMEPAD_BUTTON_UP => {
                    set_event_enabled(EventType::JOYSTICK_AXIS_MOTION, true);
                    set_event_enabled(EventType::JOYSTICK_HAT_MOTION, true);
                    set_event_enabled(EventType::JOYSTICK_BUTTON_DOWN, true);
                    set_event_enabled(EventType::JOYSTICK_BUTTON_UP, true);
                }
                EventType::GAMEPAD_UPDATE_COMPLETE => {
                    set_event_enabled(EventType::JOYSTICK_UPDATE_COMPLETE, true)
                }
                _ => {}
            }
        } else {
            // Disable this event type and discard pending events
            DISABLED_EVENTS[word].fetch_or(bit, Ordering::Relaxed);
            flush_event(event_type);
        }

        /* turn off drag'n'drop support if we've disabled the events.
        This might change some UI details at the OS level. */
        if event_type == EventType::DROP_FILE || event_type == EventType::DROP_TEXT {
            if let Some(video) = video() {
                video.toggle_drag_and_drop_support();
            }
        }
    }
}

/// The (word index, bit mask) of a type in [`DISABLED_EVENTS`], mirroring
/// upstream's `hi`/`lo` split of the type value.
fn disabled_bit(event_type: EventType) -> (usize, u32) {
    let hi = ((event_type.0 >> 8) & 0xff) as usize;
    let lo = (event_type.0 & 0xff) as usize;
    (hi * 8 + lo / 32, 1u32 << (lo & 31))
}

/// Query the state of processing events by type. Translation of `SDL_EventEnabled()`.
pub fn event_enabled(event_type: EventType) -> bool {
    let (word, bit) = disabled_bit(event_type);
    (DISABLED_EVENTS[word].load(Ordering::Relaxed) & bit) == 0
}

/// Allocate a set of user-defined events, and return the beginning event
/// number for that set of events. Returns `None` if there are not enough
/// user-defined events left. Translation of `SDL_RegisterEvents()`.
pub fn register_events(num_events: i32) -> Option<EventType> {
    if num_events > 0 {
        let value = USER_EVENTS.fetch_add(num_events, Ordering::Relaxed);
        if value >= 0 && value as u32 <= (EventType::LAST.0 - EventType::USER.0) {
            return Some(EventType(EventType::USER.0 + value as u32));
        }
    }
    None
}

/// Send an application/system event (`QUIT`, lifecycle, locale, theme, ...).
/// Translation of `SDL_SendAppEvent()`.
pub(crate) fn send_app_event(event_type: EventType) {
    if event_enabled(event_type) {
        let event = Event::simple(event_type, Duration::ZERO);

        match event_type {
            EventType::TERMINATING
            | EventType::LOW_MEMORY
            | EventType::WILL_ENTER_BACKGROUND
            | EventType::DID_ENTER_BACKGROUND
            | EventType::WILL_ENTER_FOREGROUND
            | EventType::DID_ENTER_FOREGROUND => {
                // We won't actually queue this event, it needs to be handled in this call stack by an event watcher
                if event_logging_verbosity() > 0 {
                    log_event(&event);
                }
                call_event_watchers(&event);
            }
            _ => {
                let _ = push(event);
            }
        }
    }
}

/// Translation of `SDL_SendKeymapChangedEvent()`.
pub(crate) fn send_keymap_changed_event() {
    send_app_event(EventType::KEYMAP_CHANGED);
}

/// Post `LOCALE_CHANGED` (called by the platform layer). Translation of `SDL_SendLocaleChangedEvent()`.
pub fn send_locale_changed_event() {
    send_app_event(EventType::LOCALE_CHANGED);
}

/// Post `SYSTEM_THEME_CHANGED` (called by the platform layer). Translation of `SDL_SendSystemThemeChangedEvent()`.
pub fn send_system_theme_changed_event() {
    send_app_event(EventType::SYSTEM_THEME_CHANGED);
}

pub use super::quit::send_quit;

// ---------------------------------------------------------------------------
// Init / quit
// ---------------------------------------------------------------------------

/// Translation of `SDL_InitEvents()`.
pub(crate) fn init_events() -> Result<()> {
    let mut callbacks = Vec::new();
    callbacks.push(hints::watch(hints::AUTO_UPDATE_JOYSTICKS, |c| {
        auto_update_joysticks_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::AUTO_UPDATE_SENSORS, |c| {
        auto_update_sensors_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::EVENT_LOGGING, |c| {
        event_logging_changed(c.new_value)
    })?);
    callbacks.push(hints::watch(hints::POLL_SENTINEL, |c| {
        poll_sentinel_changed(c.new_value)
    })?);
    *HINT_CALLBACKS.lock().unwrap_or_else(|e| e.into_inner()) = callbacks;

    init_main_thread_callbacks();
    if let Err(e) = start_event_loop() {
        HINT_CALLBACKS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        return Err(e);
    }

    super::quit::init_quit();

    Ok(())
}

/// Translation of `SDL_QuitEvents()`.
pub(crate) fn quit_events() {
    super::quit::quit_quit();
    stop_event_loop();
    quit_main_thread_callbacks();
    HINT_CALLBACKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

/// The number of events currently queued (including the poll sentinel).
pub fn queued_event_count() -> usize {
    EVENT_Q_COUNT.load(Ordering::Relaxed).max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{CommonEvent, UserEvent};
    use std::sync::atomic::AtomicUsize;

    /// Quits the events subsystem even if the test panics, so one failure
    /// doesn't cascade into the next test.
    struct QuitOnDrop;
    impl Drop for QuitOnDrop {
        fn drop(&mut self) {
            init::quit_subsystem(init::InitFlags::EVENTS);
        }
    }

    fn with_events<R>(f: impl FnOnce() -> R) -> R {
        let _guard = crate::test_support::test_lock();
        init::init(init::InitFlags::EVENTS).unwrap();
        let _quit = QuitOnDrop;
        f()
    }

    fn user(code: i32) -> Event {
        Event::User(UserEvent {
            event_type: EventType::USER,
            code,
            ..Default::default()
        })
    }

    #[test]
    fn push_poll_roundtrip() {
        with_events(|| {
            assert!(poll().is_none());
            assert!(push(user(1)).unwrap());
            assert!(push(user(2)).unwrap());
            assert!(has_event(EventType::USER));
            assert!(!has_event(EventType::QUIT));
            let a = poll().unwrap();
            let b = poll().unwrap();
            assert!(matches!(a, Event::User(UserEvent { code: 1, .. })));
            assert!(matches!(b, Event::User(UserEvent { code: 2, .. })));
            assert_ne!(a.timestamp(), Duration::ZERO, "push stamps zero timestamps");
            assert!(poll().is_none());
        });
    }

    #[test]
    fn peek_get_flush() {
        with_events(|| {
            for i in 0..5 {
                push(user(i)).unwrap();
            }
            send_quit();
            let peeked = peek_events(EventType::USER, EventType::USER, 3).unwrap();
            assert_eq!(peeked.len(), 3);
            assert_eq!(queued_event_count(), 6);
            let got = get_events(EventType::USER, EventType::LAST, 2).unwrap();
            assert_eq!(got.len(), 2);
            assert_eq!(queued_event_count(), 4);
            assert!(has_events(EventType::QUIT, EventType::QUIT));
            flush_events(EventType::USER, EventType::LAST);
            assert_eq!(queued_event_count(), 1);
            assert!(matches!(poll(), Some(Event::Quit(_))));
        });
    }

    #[test]
    fn poll_sentinel_ends_cycle() {
        with_events(|| {
            // A poll cycle: events pushed while polling are not returned until the next cycle.
            push(user(1)).unwrap();
            assert!(matches!(poll(), Some(Event::User(_))));
            // The pump appended a sentinel; pushing after it means the next poll
            // returns the sentinel-end (None) before the new event.
            push(user(2)).unwrap();
            assert!(poll().is_none());
            assert!(matches!(
                poll(),
                Some(Event::User(UserEvent { code: 2, .. }))
            ));
            assert!(poll().is_none());
        });
    }

    #[test]
    fn filter_and_watch() {
        with_events(|| {
            let seen = Arc::new(AtomicUsize::new(0));
            let s2 = seen.clone();
            let watch = add_watch(move |_| {
                s2.fetch_add(1, Ordering::Relaxed);
            });
            push(user(1)).unwrap();
            push(user(2)).unwrap();
            // Filter rejects odd codes, and is applied to the already-queued events.
            set_filter(|e| !matches!(e, Event::User(UserEvent { code, .. }) if code % 2 == 1));
            assert!(has_filter());
            assert_eq!(queued_event_count(), 1);
            assert!(!push(user(3)).unwrap(), "filtered events are not queued");
            assert!(push(user(4)).unwrap());
            assert_eq!(
                seen.load(Ordering::Relaxed),
                3,
                "filtered events never reach watchers"
            );
            watch.remove();
            push(user(6)).unwrap();
            assert_eq!(seen.load(Ordering::Relaxed), 3);
            clear_filter();
            assert!(!has_filter());
            filter_events(|e| matches!(e, Event::User(UserEvent { code: 6, .. })));
            assert_eq!(queued_event_count(), 1);
            assert!(matches!(
                poll(),
                Some(Event::User(UserEvent { code: 6, .. }))
            ));
        });
    }

    #[test]
    fn watcher_can_remove_itself_during_dispatch() {
        with_events(|| {
            let slot: Arc<Mutex<Option<EventWatch>>> = Arc::new(Mutex::new(None));
            let s2 = slot.clone();
            let count = Arc::new(AtomicUsize::new(0));
            let c2 = count.clone();
            let watch = add_watch(move |_| {
                c2.fetch_add(1, Ordering::Relaxed);
                if let Some(w) = s2.lock().unwrap().take() {
                    w.remove();
                }
            });
            *slot.lock().unwrap() = Some(watch);
            push(user(1)).unwrap();
            push(user(2)).unwrap();
            assert_eq!(count.load(Ordering::Relaxed), 1);
        });
    }

    #[test]
    fn enable_disable() {
        with_events(|| {
            assert!(event_enabled(EventType::KEY_DOWN));
            push(Event::Key(Default::default())).unwrap();
            set_event_enabled(EventType::KEY_UP, false);
            assert!(!event_enabled(EventType::KEY_UP));
            assert!(event_enabled(EventType::KEY_DOWN));
            assert_eq!(
                queued_event_count(),
                0,
                "disabling flushes pending events of that type"
            );
            set_event_enabled(EventType::KEY_UP, true);
            assert!(event_enabled(EventType::KEY_UP));

            // Gamepad events depend on joystick events
            set_event_enabled(EventType::JOYSTICK_ADDED, false);
            set_event_enabled(EventType::GAMEPAD_ADDED, false);
            set_event_enabled(EventType::GAMEPAD_ADDED, true);
            assert!(event_enabled(EventType::JOYSTICK_ADDED));

            // The poll sentinel hint
            hints::set(hints::POLL_SENTINEL, "0").unwrap();
            assert!(!event_enabled(EventType::POLL_SENTINEL));
            hints::reset(hints::POLL_SENTINEL);
            assert!(event_enabled(EventType::POLL_SENTINEL));
        });
    }

    #[test]
    fn user_event_registration() {
        let a = register_events(2).unwrap();
        let b = register_events(1).unwrap();
        assert!(a.is_user() && b.is_user());
        assert!(b.0 >= a.0 + 2);
        assert_eq!(register_events(0), None);
        assert_eq!(register_events(-1), None);
    }

    #[test]
    fn app_events_and_lifecycle() {
        with_events(|| {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let s2 = seen.clone();
            let _watch = add_watch(move |e| s2.lock().unwrap().push(e.event_type()));
            send_app_event(EventType::LOW_MEMORY);
            send_keymap_changed_event();
            // Lifecycle events are delivered to watchers only, never queued.
            assert_eq!(queued_event_count(), 1);
            assert_eq!(
                *seen.lock().unwrap(),
                vec![EventType::LOW_MEMORY, EventType::KEYMAP_CHANGED]
            );
            assert!(matches!(
                poll(),
                Some(Event::App(CommonEvent {
                    event_type: EventType::KEYMAP_CHANGED,
                    ..
                }))
            ));
        });
    }

    #[test]
    fn wait_timeout_and_shutdown() {
        with_events(|| {
            let start = std::time::Instant::now();
            assert!(wait_timeout(Some(Duration::from_millis(20)))
                .unwrap()
                .is_none());
            assert!(start.elapsed() >= Duration::from_millis(15));
            push(user(9)).unwrap();
            assert!(matches!(wait(), Ok(Event::User(_))));
        });
        let _guard = crate::test_support::test_lock();
        // After shutdown the queue refuses to add or hand out events.
        let err = get_events(EventType::FIRST, EventType::LAST, 1).unwrap_err();
        assert_eq!(err.message(), "The event system has been shut down");
        assert!(push(user(1)).is_err());
        assert!(!has_event(EventType::USER));
    }

    #[test]
    fn main_thread_callbacks() {
        with_events(|| {
            let ran = Arc::new(AtomicUsize::new(0));
            // On the main thread the callback runs inline.
            let r2 = ran.clone();
            run_on_main_thread(
                move || {
                    r2.fetch_add(1, Ordering::Relaxed);
                },
                true,
            )
            .unwrap();
            assert_eq!(ran.load(Ordering::Relaxed), 1);

            // From another thread it is queued until the main thread pumps.
            let r3 = ran.clone();
            let t = std::thread::spawn(move || {
                run_on_main_thread(
                    move || {
                        r3.fetch_add(1, Ordering::Relaxed);
                    },
                    true,
                )
            });
            while main_callbacks().is_empty() {
                std::thread::yield_now();
            }
            assert_eq!(ran.load(Ordering::Relaxed), 1);
            pump();
            assert_eq!(ran.load(Ordering::Relaxed), 2);
            t.join().unwrap().unwrap();

            // Quitting cancels waiting callbacks.
            let t = std::thread::spawn(|| run_on_main_thread(|| {}, true));
            while main_callbacks().is_empty() {
                std::thread::yield_now();
            }
            quit_main_thread_callbacks();
            let err = t.join().unwrap().unwrap_err();
            assert_eq!(err.message(), "Callback canceled");
        });
    }
}
