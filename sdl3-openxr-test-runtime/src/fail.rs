// The test runtime's failure injection.
// Part of the repository's own test code (not a translation); see LICENSE.txt.

//! [`FAIL`] (`SDL3_OPENXR_TEST_RUNTIME_FAIL`) lists the calls that fail,
//! separated by commas, each `POINT[@N][=RESULT]`:
//!
//! * `POINT` is the name of a function the runtime serves
//!   (`xrCreateInstance`, `xrGetSystem`, `xrCreateVulkanInstanceKHR`...),
//!   whose call then returns `RESULT` at once (before looking at its
//!   arguments, so with nothing written to them); or
//!   `xrGetInstanceProcAddr:NAME`, the lookup of the function `NAME`; or
//!   the name of an extension (`XR_KHR_vulkan_enable2`), which the runtime
//!   then doesn't have (not listed, not enabled);
//! * `@N` makes only the `N`th call (from 1) fail, counting the calls
//!   (lookups) of that point since the library was loaded; without it,
//!   every call does;
//! * `RESULT` is the `XrResult` returned, `XR_ERROR_RUNTIME_FAILURE` (-2)
//!   by default (`XR_ERROR_FUNCTION_UNSUPPORTED`, -7, for a lookup); for
//!   `xrCreateVulkanInstanceKHR` and `xrCreateVulkanDeviceKHR` it can be
//!   `vk:VKRESULT` instead: `XR_SUCCESS`, with that `VkResult` as the
//!   Vulkan call's (which isn't made).
//!
//! A list that can't be parsed ends the process (with a message), so that
//! a test never runs without the failures it asked for. The injected
//! failures are written to the standard error; with [`LOG`]
//! (`SDL3_OPENXR_TEST_RUNTIME_LOG=1`) every call is.

use std::sync::{Mutex, OnceLock};

use crate::sys::*;

/// The variable listing the failures.
pub const FAIL: &str = "SDL3_OPENXR_TEST_RUNTIME_FAIL";
/// The variable that logs every call to the standard error (`1`).
pub const LOG: &str = "SDL3_OPENXR_TEST_RUNTIME_LOG";

/// What a failing call does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Return this result.
    Xr(XrResult),
    /// (`xrCreateVulkanInstanceKHR`, `xrCreateVulkanDeviceKHR`) return
    /// `XR_SUCCESS`, with this `VkResult`.
    Vk(VkResult),
}

/// One failure of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub point: String,
    pub nth: Option<u32>,
    pub outcome: Outcome,
}

/// The failure list, parsed.
pub fn parse(list: &str) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::new();
    for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        let (left, result) = match item.split_once('=') {
            Some((left, result)) => (left, Some(result)),
            None => (item, None),
        };
        let (point, nth) = match left.split_once('@') {
            Some((point, n)) => match n.parse::<u32>() {
                Ok(n) if n > 0 => (point, Some(n)),
                _ => return Err(format!("{item}: the call number isn't a number from 1")),
            },
            None => (left, None),
        };
        if point.is_empty() {
            return Err(format!("{item}: no point"));
        }
        let vulkan = matches!(
            point,
            "xrCreateVulkanInstanceKHR" | "xrCreateVulkanDeviceKHR"
        );
        let outcome = match result {
            None if point.starts_with("xrGetInstanceProcAddr:") => {
                Outcome::Xr(XR_ERROR_FUNCTION_UNSUPPORTED)
            }
            None => Outcome::Xr(XR_ERROR_RUNTIME_FAILURE),
            Some(r) => match r.strip_prefix("vk:") {
                Some(vk) if vulkan => Outcome::Vk(
                    vk.parse()
                        .map_err(|_| format!("{item}: not a VkResult: {vk}"))?,
                ),
                Some(_) => return Err(format!("{item}: vk: is for the Vulkan wrappers only")),
                None => Outcome::Xr(
                    r.parse()
                        .map_err(|_| format!("{item}: not an XrResult: {r}"))?,
                ),
            },
        };
        if !point.starts_with("xr") && !point.starts_with("XR_") {
            return Err(format!("{item}: not a function or an extension: {point}"));
        }
        rules.push(Rule {
            point: point.to_owned(),
            nth,
            outcome,
        });
    }
    Ok(rules)
}

/// The list of [`FAIL`], parsed once.
fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let list = std::env::var(FAIL).unwrap_or_default();
        match parse(&list) {
            Ok(rules) => rules,
            Err(e) => {
                eprintln!("sdl3-openxr-test-runtime: {FAIL}: {e}");
                std::process::abort();
            }
        }
    })
}

/// Whether every call is logged.
pub fn logging() -> bool {
    static LOGGING: OnceLock<bool> = OnceLock::new();
    *LOGGING.get_or_init(|| std::env::var(LOG).is_ok_and(|v| v == "1"))
}

/// The calls of each point so far.
static CALLS: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());

/// A call of `point`: counted, logged, and its injected failure if the
/// list has one for this call.
pub fn call(point: &str) -> Option<Outcome> {
    let n = {
        let mut calls = CALLS.lock().unwrap_or_else(|e| e.into_inner());
        match calls.iter_mut().find(|(p, _)| p == point) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                calls.push((point.to_owned(), 1));
                1
            }
        }
    };
    let outcome = rules()
        .iter()
        .find(|r| r.point == point && r.nth.is_none_or(|nth| nth == n))
        .map(|r| r.outcome);
    if let Some(outcome) = outcome {
        eprintln!("sdl3-openxr-test-runtime: {point} (call {n}): injected {outcome:?}");
    } else if logging() {
        eprintln!("sdl3-openxr-test-runtime: {point} (call {n})");
    }
    outcome
}

/// Whether the list removes an extension.
pub fn extension_removed(name: &str) -> bool {
    rules().iter().any(|r| r.point == name)
}

/// [`call`] for a function that only fails with an `XrResult` (a `vk:`
/// outcome is only parsed for the Vulkan wrappers).
pub fn xr_call(point: &str) -> Option<XrResult> {
    match call(point) {
        Some(Outcome::Xr(result)) => Some(result),
        Some(Outcome::Vk(_)) | None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_points_calls_and_results() {
        assert_eq!(parse("").unwrap(), vec![]);
        assert_eq!(
            parse("xrGetSystem, xrCreateInstance@2=-6,xrGetInstanceProcAddr:xrDestroySwapchain")
                .unwrap(),
            vec![
                Rule {
                    point: "xrGetSystem".into(),
                    nth: None,
                    outcome: Outcome::Xr(XR_ERROR_RUNTIME_FAILURE)
                },
                Rule {
                    point: "xrCreateInstance".into(),
                    nth: Some(2),
                    outcome: Outcome::Xr(XR_ERROR_INITIALIZATION_FAILED)
                },
                Rule {
                    point: "xrGetInstanceProcAddr:xrDestroySwapchain".into(),
                    nth: None,
                    outcome: Outcome::Xr(XR_ERROR_FUNCTION_UNSUPPORTED)
                },
            ]
        );
        assert_eq!(
            parse("xrCreateVulkanDeviceKHR=vk:-3").unwrap()[0].outcome,
            Outcome::Vk(-3)
        );
        assert_eq!(
            parse("XR_KHR_vulkan_enable2").unwrap()[0].point,
            "XR_KHR_vulkan_enable2"
        );
        for bad in [
            "xrGetSystem@0",
            "xrGetSystem@x",
            "xrGetSystem=x",
            "xrGetSystem=vk:-3",
            "@2",
            "getSystem",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
