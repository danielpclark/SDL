// The events the Android video programs watch, and their trips to the
// background.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An event watch keeps the application's, display, window and render
//! events (their types and data) as they're pushed (from SDL's thread, or
//! the activity's, for the surface's), for the programs to trace: in
//! order where SDL's thread sends them, as a set of names where the
//! activity's thread does, whose order against SDL's is a matter of
//! timing. A trip to the background ([`trip`]) brings the app back from
//! the watch, as the pump blocks while the app is paused.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sdl3::events::{self, Event, EventType, EventWatch};

use super::{event_name, request};

/// An event the watch kept: its type and data (a window event's, a
/// display event's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    pub ty: EventType,
    pub data1: i32,
    pub data2: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
static WATCH: Mutex<Option<EventWatch>> = Mutex::new(None);
static FOREGROUND_ON_BACKGROUND: AtomicBool = AtomicBool::new(false);

/// Whether the watch keeps events of this type.
fn kept(ty: EventType) -> bool {
    // (the application's, the displays' and the windows', 0x100 to 0x2ff,
    // and the renderers', 0x2000 to 0x20ff)
    (0x100..0x300).contains(&ty.0) || (0x2000..0x2100).contains(&ty.0)
}

/// Start watching.
pub fn watch() {
    *WATCH.lock().unwrap() = Some(events::add_watch(|event| {
        let ty = event.event_type();
        if !kept(ty) {
            return;
        }
        let (data1, data2) = match event {
            Event::Window(e) => (e.data1, e.data2),
            Event::Display(e) => (e.data1, e.data2),
            _ => (0, 0),
        };
        SEEN.lock().unwrap().push(Seen { ty, data1, data2 });
        // (from the watch, before the pump blocks while in the background)
        if ty == EventType::DID_ENTER_BACKGROUND && FOREGROUND_ON_BACKGROUND.load(Ordering::SeqCst)
        {
            request("foreground");
        }
    }));
}

/// Stop watching.
pub fn unwatch() {
    drop(WATCH.lock().unwrap().take());
}

/// The events the watch kept since the last call.
pub fn take() -> Vec<Seen> {
    std::mem::take(&mut *SEEN.lock().unwrap())
}

/// Whether the watch has kept an event of type `ty` since the last
/// [`take`].
pub fn saw(ty: EventType) -> bool {
    SEEN.lock().unwrap().iter().any(|s| s.ty == ty)
}

/// Events in order, `NAME(data1,data2)`, separated by commas, those `keep`
/// keeps (`(none)` for none).
pub fn ordered(events: &[Seen], keep: impl Fn(EventType) -> bool) -> String {
    let items: Vec<String> = events
        .iter()
        .filter(|s| keep(s.ty))
        .map(|s| format!("{}({},{})", event_name(s.ty), s.data1, s.data2))
        .collect();
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.join(", ")
    }
}

/// The names of the events' types, sorted, each once (`(none)` for none),
/// those `keep` keeps.
pub fn names(events: &[Seen], keep: impl Fn(EventType) -> bool) -> String {
    let mut items: Vec<String> = events
        .iter()
        .filter(|s| keep(s.ty))
        .map(|s| event_name(s.ty))
        .collect();
    items.sort();
    items.dedup();
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.join(", ")
    }
}

/// Whether `ty` is one of the application's events.
pub fn is_app_event(ty: EventType) -> bool {
    (0x100..0x150).contains(&ty.0)
}

/// Pump the events for `ms`.
pub fn pump_for(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        events::pump();
        events::flush_events(EventType::FIRST, EventType::LAST);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Pump the events until `done` (or `ms` have gone), then `settle` ms
/// more: whether `done` came.
pub fn pump_until(done: impl Fn() -> bool, ms: u64, settle: u64) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    let mut came = false;
    while Instant::now() < end {
        events::pump();
        events::flush_events(EventType::FIRST, EventType::LAST);
        if done() {
            came = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    pump_for(settle);
    came
}

/// A trip to the background (the HOME key) and back (the activity
/// launched again, from the watch once the app has entered the
/// background): the events the watch kept meanwhile (until
/// `SDL_EVENT_DID_ENTER_FOREGROUND`, then `settle` ms more).
pub fn trip(settle: u64) -> Vec<Seen> {
    FOREGROUND_ON_BACKGROUND.store(true, Ordering::SeqCst);
    request("background");
    pump_until(|| saw(EventType::DID_ENTER_FOREGROUND), 30_000, settle);
    FOREGROUND_ON_BACKGROUND.store(false, Ordering::SeqCst);
    take()
}
