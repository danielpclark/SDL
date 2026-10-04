// Rust translation of src/core/linux/SDL_dbus.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Test support for the D-Bus layer and its consumers: a private bus run by
//! `dbus-daemon` with its own configuration, peers that answer method
//! calls on it, and pointing SDL's session context at it.

use super::{lib, state, Connection, Context, HandlerResult, Message};
use crate::test_support::TempDir;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The address [`Connection::session_private`] connects to instead of the
/// real session bus (set by [`use_as_session`]).
static SESSION_ADDRESS: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn session_address() -> Option<String> {
    SESSION_ADDRESS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// A private session bus for a test, with its own configuration in a temp
/// directory; killed on drop.
pub(crate) struct Bus {
    child: Child,
    pub(crate) address: String,
    _dir: TempDir,
}

impl Bus {
    /// Start a bus, or `None` (after printing why) without libdbus or
    /// `dbus-daemon`.
    pub(crate) fn start() -> Option<Bus> {
        if lib().is_none() {
            println!("libdbus isn't available; skipping");
            return None;
        }
        let dir = TempDir::new("dbus");
        let config = dir.path("session.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                dir.0
            ),
        )
        .ok()?;
        let mut child = match Command::new("dbus-daemon")
            .arg(format!("--config-file={config}"))
            .args(["--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                println!("dbus-daemon isn't installed; skipping");
                return None;
            }
        };
        let mut line = String::new();
        BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .ok()?;
        if line.trim().is_empty() {
            println!("dbus-daemon didn't start; skipping");
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        Some(Bus {
            child,
            address: line.trim().to_owned(),
            _dir: dir,
        })
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A peer answering method calls on another thread until it is dropped.
pub(crate) struct Peer {
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Own `name` on the bus at `address` and answer what `handler` answers,
/// from another thread; stopped with [`Peer::stop`] or on drop.
pub(crate) fn peer(
    address: &str,
    name: &'static str,
    handler: impl Fn(&Arc<Connection>, &Message) -> Option<Message> + Send + 'static,
) -> Peer {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let address = address.to_owned();
    let thread = std::thread::spawn(move || {
        let conn = Arc::new(Connection::open_address(&address).unwrap());
        assert_eq!(conn.request_name(name, 0).unwrap(), 1, "primary owner");
        let replies = Arc::new(Mutex::new(Vec::new()));
        let r2 = replies.clone();
        let c2 = Arc::downgrade(&conn);
        let _filter = conn
            .add_filter(move |msg| {
                let Some(conn) = c2.upgrade() else {
                    return HandlerResult::NotYetHandled;
                };
                match handler(&conn, msg) {
                    Some(reply) => {
                        r2.lock().unwrap().push(reply);
                        HandlerResult::Handled
                    }
                    None => HandlerResult::NotYetHandled,
                }
            })
            .unwrap();
        tx.send(()).unwrap();
        while !s2.load(Ordering::SeqCst) {
            conn.read_write_dispatch(20);
            let pending: Vec<Message> = replies.lock().unwrap().drain(..).collect();
            for reply in pending {
                conn.send(&reply);
            }
        }
    });
    rx.recv().unwrap();
    Peer {
        thread: Some(thread),
        stop,
    }
}

impl Peer {
    /// Stop answering and wait for the thread.
    pub(crate) fn stop(self) {
        drop(self);
    }
}

/// [`peer`], returning the join handle and stop flag the D-Bus layer's own
/// tests use.
pub(crate) fn serve(
    address: String,
    name: &'static str,
    handler: impl Fn(&Message) -> Option<Message> + Send + 'static,
) -> (std::thread::JoinHandle<()>, Arc<AtomicBool>) {
    let mut p = peer(&address, name, move |_, msg| handler(msg));
    let t = p.thread.take().unwrap();
    let stop = p.stop.clone();
    std::mem::forget(p);
    (t, stop)
}

/// Point SDL's session context (and [`Connection::session_private`]) at
/// `bus` until [`release_session`]. Callers hold the test lock.
pub(crate) fn use_as_session(bus: &Bus) {
    super::quit();
    let mut s = state();
    s.is_dbus_available = true;
    s.interface_unavailable = false;
    let session_conn = Connection::open_address(&bus.address).unwrap();
    s.context = Some(Arc::new(Context {
        session_conn,
        system_conn: None,
    }));
    s.initialized = true;
    *SESSION_ADDRESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(bus.address.clone());
}

/// Undo [`use_as_session`]: close the context and forget the address.
pub(crate) fn release_session() {
    super::quit();
    *SESSION_ADDRESS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // (let the real session bus be tried again)
    state().is_dbus_available = true;
}

/// Wait up to five seconds for `cond`, pumping with `pump` meanwhile.
pub(crate) fn wait_for(mut pump: impl FnMut(), mut cond: impl FnMut() -> bool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        pump();
        if cond() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cond()
}
