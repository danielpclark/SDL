// Rust translation of src/core/linux/SDL_threadprio.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Linux thread priorities: `setpriority()` nice levels, with RealtimeKit
//! (directly on the system bus, or through the desktop portal) as the
//! fallback for raising priorities and for realtime scheduling.

use super::dbus::{self, Arg, Connection, Value};
use crate::error::{Error, Result};
use crate::thread::ThreadPriority;
use std::sync::OnceLock;

// RLIMIT_RTTIME requires kernel >= 2.6.25 and is in glibc >= 2.14
const RLIMIT_RTTIME: libc::c_int = 15;
// SCHED_RESET_ON_FORK is in kernel >= 2.6.32.
const SCHED_RESET_ON_FORK: libc::c_int = 0x4000_0000;

// d-bus queries to org.freedesktop.RealtimeKit1.
const RTKIT_DBUS_NODE: &str = "org.freedesktop.RealtimeKit1";
const RTKIT_DBUS_PATH: &str = "/org/freedesktop/RealtimeKit1";
const RTKIT_DBUS_INTERFACE: &str = "org.freedesktop.RealtimeKit1";

// d-bus queries to the XDG portal interface to RealtimeKit1
const XDG_PORTAL_DBUS_NODE: &str = "org.freedesktop.portal.Desktop";
const XDG_PORTAL_DBUS_PATH: &str = "/org/freedesktop/portal/desktop";
const XDG_PORTAL_DBUS_INTERFACE: &str = "org.freedesktop.portal.Realtime";

static RTKIT: OnceLock<Rtkit> = OnceLock::new();

/// What `rtkit_initialize()` found out, once per process.
struct Rtkit {
    use_session_conn: bool,
    node: &'static str,
    path: &'static str,
    interface: &'static str,
    min_nice_level: i32,
    max_realtime_priority: i32,
    max_rttime_usec: i64,
}

/// Checking that the RTTimeUSecMax property exists and is an int64
/// confirms that the desktop portal exists and supports the realtime
/// interface, and that the interface is new enough to have the required
/// bug fixes applied. Translation of `realtime_portal_supported()`.
fn realtime_portal_supported(conn: &Connection) -> bool {
    matches!(
        dbus::query_property_on_connection(
            conn,
            XDG_PORTAL_DBUS_NODE,
            XDG_PORTAL_DBUS_PATH,
            XDG_PORTAL_DBUS_INTERFACE,
            "RTTimeUSecMax"
        ),
        Some(Value::I64(_))
    )
}

/// Translation of `get_rtkit_dbus_connection()`.
fn rtkit_connection<'a>(ctx: &'a dbus::Context, rtkit: &Rtkit) -> Option<&'a Connection> {
    if rtkit.use_session_conn {
        Some(&ctx.session_conn)
    } else {
        ctx.system_conn.as_ref()
    }
}

/// Translation of `set_rtkit_interface()` and `rtkit_initialize()`.
fn rtkit() -> &'static Rtkit {
    RTKIT.get_or_init(|| {
        let ctx = dbus::context();
        // xdg-desktop-portal works in all instances, so check for it first.
        let portal = ctx
            .as_ref()
            .is_some_and(|c| realtime_portal_supported(&c.session_conn));
        let mut r = if portal {
            Rtkit {
                use_session_conn: true,
                node: XDG_PORTAL_DBUS_NODE,
                path: XDG_PORTAL_DBUS_PATH,
                interface: XDG_PORTAL_DBUS_INTERFACE,
                min_nice_level: -20,
                max_realtime_priority: 99,
                max_rttime_usec: 200000,
            }
        } else {
            // Fall back to the standard rtkit interface in all other cases.
            Rtkit {
                use_session_conn: false,
                node: RTKIT_DBUS_NODE,
                path: RTKIT_DBUS_PATH,
                interface: RTKIT_DBUS_INTERFACE,
                min_nice_level: -20,
                max_realtime_priority: 99,
                max_rttime_usec: 200000,
            }
        };
        let Some(ctx) = ctx else { return r };
        let Some(conn) = rtkit_connection(&ctx, &r) else {
            return r;
        };
        let query =
            |prop| dbus::query_property_on_connection(conn, r.node, r.path, r.interface, prop);

        // Try getting minimum nice level: this is often greater than PRIO_MIN (-20).
        if let Some(Value::I32(v)) = query("MinNiceLevel") {
            r.min_nice_level = v;
        }
        // Try getting maximum realtime priority: this can be less than the POSIX default (99).
        if let Some(Value::I32(v)) = query("MaxRealtimePriority") {
            r.max_realtime_priority = v;
        }
        // Try getting maximum rttime allowed by rtkit: exceeding this value will result in SIGKILL
        if let Some(Value::I64(v)) = query("RTTimeUSecMax") {
            r.max_rttime_usec = v;
        }
        r
    })
}

/// Meet RealtimeKit's requirements for this thread: RLIMIT_RTTIME set and
/// SCHED_RESET_ON_FORK in the scheduler policy. Translation of
/// `rtkit_initialize_realtime_thread()`.
fn rtkit_initialize_realtime_thread(rtkit: &Rtkit) -> bool {
    // Following is an excerpt from rtkit README that outlines the requirements
    // a thread must meet before making rtkit requests:
    //
    //   * Only clients with RLIMIT_RTTIME set will get RT scheduling
    //
    //   * RT scheduling will only be handed out to processes with
    //     SCHED_RESET_ON_FORK set to guarantee that the scheduling
    //     settings cannot 'leak' to child processes, thus making sure
    //     that 'RT fork bombs' cannot be used to bypass RLIMIT_RTTIME
    //     and take the system down.
    //
    //   * Limits are enforced on all user controllable resources, only
    //     a maximum number of users, processes, threads can request RT
    //     scheduling at the same time.
    //
    //   * Only a limited number of threads may be made RT in a
    //     specific time frame.
    //
    //   * Client authorization is verified with PolicyKit

    let n_pid: libc::pid_t = 0; // self
                                // SAFETY: plain syscalls on this thread with valid out-structs.
    unsafe {
        let n_sched_policy = libc::sched_getscheduler(n_pid) | SCHED_RESET_ON_FORK;
        let mut sched_param: libc::sched_param = std::mem::zeroed();

        // Requirement #1: Set RLIMIT_RTTIME
        let mut rlimit: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(RLIMIT_RTTIME as _, &mut rlimit) != 0 {
            return false;
        }

        // Current rtkit allows a max of 200ms right now
        rlimit.rlim_max = rtkit.max_rttime_usec as libc::rlim_t;
        rlimit.rlim_cur = rlimit.rlim_max / 2;
        if libc::setrlimit(RLIMIT_RTTIME as _, &rlimit) != 0 {
            return false;
        }

        // Requirement #2: Add SCHED_RESET_ON_FORK to the scheduler policy
        if libc::sched_getparam(n_pid, &mut sched_param) != 0 {
            return false;
        }
        libc::sched_setscheduler(n_pid, n_sched_policy, &sched_param) == 0
    }
}

/// Translation of `rtkit_setpriority_nice()`.
fn rtkit_setpriority_nice(thread: i64, nice_level: i32) -> bool {
    let rtkit = rtkit();
    let Some(ctx) = dbus::context() else {
        return false;
    };
    let Some(conn) = rtkit_connection(&ctx, rtkit) else {
        return false;
    };
    // SAFETY: getpid has no preconditions.
    let pid = unsafe { libc::getpid() } as u64;
    let nice = nice_level.max(rtkit.min_nice_level);
    dbus::call_method_on_connection(
        conn,
        rtkit.node,
        rtkit.path,
        rtkit.interface,
        "MakeThreadHighPriorityWithPID",
        &[Arg::U64(pid), Arg::U64(thread as u64), Arg::I32(nice)],
    )
    .is_some()
}

/// Translation of `rtkit_setpriority_realtime()`.
fn rtkit_setpriority_realtime(thread: i64, rt_priority: i32) -> bool {
    let rtkit = rtkit();
    let Some(ctx) = dbus::context() else {
        return false;
    };
    let Some(conn) = rtkit_connection(&ctx, rtkit) else {
        return false;
    };
    // SAFETY: getpid has no preconditions.
    let pid = unsafe { libc::getpid() } as u64;
    let priority = (rt_priority as u32).min(rtkit.max_realtime_priority as u32);

    // We always perform the thread state changes necessary for rtkit.
    // This wastes some system calls if the state is already set but
    // typically code sets a thread priority and leaves it so it's
    // not expected that this wasted effort will be an issue.
    // We also do not quit if this fails, we let the rtkit request
    // go through to determine whether it really needs to fail or not.
    rtkit_initialize_realtime_thread(rtkit);

    dbus::call_method_on_connection(
        conn,
        rtkit.node,
        rtkit.path,
        rtkit.interface,
        "MakeThreadRealtimeWithPID",
        &[Arg::U64(pid), Arg::U64(thread as u64), Arg::U32(priority)],
    )
    .is_some()
}

/// Set a Linux thread's nice level, asking RealtimeKit if `setpriority()`
/// is refused. Translation of `SDL_SetLinuxThreadPriority()`.
pub(crate) fn set_linux_thread_priority(thread_id: i64, priority: i32) -> Result<()> {
    // SAFETY: setpriority takes plain integers.
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, thread_id as libc::id_t, priority) } == 0 {
        return Ok(());
    }

    /* Note that this fails you most likely:
         * Have your process's scheduler incorrectly configured.
           See the requirements at:
           http://git.0pointer.net/rtkit.git/tree/README#n16
         * Encountered dbus/polkit security restrictions. Note
           that the RealtimeKit1 dbus endpoint is inaccessible
           over ssh connections for most common distro configs.
           You might want to check your local config for details:
           /usr/share/polkit-1/actions/org.freedesktop.RealtimeKit1.policy

       README and sample code at: http://git.0pointer.net/rtkit.git
    */
    if rtkit_setpriority_nice(thread_id, priority) {
        return Ok(());
    }

    Err(Error::new("setpriority() failed"))
}

/// Set a Linux thread's priority for a scheduler policy: a realtime
/// priority from RealtimeKit for `SCHED_RR`/`SCHED_FIFO`, else a nice
/// level. Translation of `SDL_SetLinuxThreadPriorityAndPolicy()`.
pub(crate) fn set_linux_thread_priority_and_policy(
    thread_id: i64,
    sdl_priority: ThreadPriority,
    sched_policy: i32,
) -> Result<()> {
    let realtime = sched_policy == libc::SCHED_RR || sched_policy == libc::SCHED_FIFO;
    let os_priority = if realtime {
        // (rtkit_max_realtime_priority: 99 until a RealtimeKit request has
        // queried the real maximum)
        let max = RTKIT.get().map_or(99, |r| r.max_realtime_priority);
        match sdl_priority {
            ThreadPriority::Low => 1,
            ThreadPriority::High => max * 3 / 4,
            ThreadPriority::TimeCritical => max,
            ThreadPriority::Normal => max / 2,
        }
    } else {
        let os_priority = match sdl_priority {
            ThreadPriority::Low => 19,
            ThreadPriority::High => -10,
            ThreadPriority::TimeCritical => -20,
            ThreadPriority::Normal => 0,
        };
        // SAFETY: setpriority takes plain integers.
        if unsafe { libc::setpriority(libc::PRIO_PROCESS, thread_id as libc::id_t, os_priority) }
            == 0
        {
            return Ok(());
        }
        os_priority
    };

    /* Note that this fails you most likely:
     * Have your process's scheduler incorrectly configured.
       See the requirements at:
       http://git.0pointer.net/rtkit.git/tree/README#n16
     * Encountered dbus/polkit security restrictions. Note
       that the RealtimeKit1 dbus endpoint is inaccessible
       over ssh connections for most common distro configs.
       You might want to check your local config for details:
       /usr/share/polkit-1/actions/org.freedesktop.RealtimeKit1.policy

       README and sample code at: http://git.0pointer.net/rtkit.git
    */
    let granted = if realtime {
        rtkit_setpriority_realtime(thread_id, os_priority)
    } else {
        rtkit_setpriority_nice(thread_id, os_priority)
    };
    if granted {
        return Ok(());
    }

    Err(Error::new("setpriority() failed"))
}
