// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLSensorManager.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class SDLSensorManager`: this class coordinates synchronized access to
//! sensor manager registration. This prevents a
//! `java.util.ConcurrentModificationException` exception on Android 16,
//! specifically on the Samsung Tab S9 Ultra: a registration that throws it
//! is tried again (after a millisecond), up to [`RETRY_COUNT`] times.
//! The singleton's `synchronized` methods are under one lock here.

use std::sync::Mutex;

use super::super::env::{Arg, Env, JniResult, Obj};
use super::super::jni::*;
use super::super::ndk::LogPriority;

/// The singleton manager (`mManager`), whose monitor its synchronized
/// methods take.
static M_MANAGER: Mutex<()> = Mutex::new(());

const RETRY_COUNT: i32 = 3;

/// Translation of `registerListener()` (and of `RegisterListener()`).
pub(crate) fn register_listener<'a>(
    env: Env<'a>,
    manager: Obj<'_>,
    listener: Obj<'_>,
    sensor: Obj<'_>,
    sampling_period_us: jint,
) -> JniResult<'a, ()> {
    let _synchronized = M_MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    retried(
        env,
        "Multiple ConcurrentModificationException caught while registering sensor listener, canceling operation",
        || {
            env.call_method(
                manager,
                c"registerListener",
                c"(Landroid/hardware/SensorEventListener;Landroid/hardware/Sensor;ILandroid/os/Handler;)Z",
                &[
                    Arg::Object(listener),
                    Arg::Object(sensor),
                    Arg::Int(sampling_period_us),
                    Arg::Object(Obj::NULL),
                ],
            )
            .map(drop)
        },
    )
}

/// Translation of `unregisterListener()` (and of `UnregisterListener()`).
pub(crate) fn unregister_listener<'a>(
    env: Env<'a>,
    manager: Obj<'_>,
    listener: Obj<'_>,
    sensor: Obj<'_>,
) -> JniResult<'a, ()> {
    let _synchronized = M_MANAGER.lock().unwrap_or_else(|e| e.into_inner());
    retried(
        env,
        "Multiple ConcurrentModificationException caught while unregistering sensor listener, canceling operation",
        || {
            env.call_method(
                manager,
                c"unregisterListener",
                c"(Landroid/hardware/SensorEventListener;Landroid/hardware/Sensor;)V",
                &[Arg::Object(listener), Arg::Object(sensor)],
            )
            .map(drop)
        },
    )
}

/// The loop of both: `call` until it doesn't throw a
/// `ConcurrentModificationException`, sleeping a bit (`Thread.sleep(1)`)
/// and trying again, up to [`RETRY_COUNT`] more times, then giving up
/// (logging `giving_up`). Other exceptions are thrown.
fn retried<'a>(
    env: Env<'a>,
    giving_up: &str,
    mut call: impl FnMut() -> JniResult<'a, ()>,
) -> JniResult<'a, ()> {
    let mut retries = 0;
    let mut complete = false;
    while !complete {
        match call() {
            Ok(()) => complete = true,
            Err(thrown) => {
                // catch (java.util.ConcurrentModificationException e)
                drop(thrown.catch_any(&[c"java/util/ConcurrentModificationException"])?);
                retries += 1;
                if retries <= RETRY_COUNT {
                    // Sleep a bit and try again
                    let thread = env.find_class(c"java/lang/Thread")?;
                    if let Err(thrown) =
                        env.call_static_method(thread.obj(), c"sleep", c"(J)V", &[Arg::Long(1)])
                    {
                        // catch (Exception e2)
                        drop(thrown.catch_exception()?);
                    }
                } else {
                    super::super::ndk::log_write(LogPriority::Verbose, "SDL", giving_up);
                    complete = true;
                }
            }
        }
    }
    Ok(())
}
