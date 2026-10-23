//! Throwaway iOS-simulator spike crate (libtest, doctest and runner probes).
//!
//! ```
//! // A doctest: does cargo run it through the target runner?
//! println!("doctest stdout line");
//! assert_eq!(ios_spike::answer(), 42);
//! ```

/// The answer.
pub fn answer() -> i32 {
    42
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit_passes() {
        println!("unit_passes stdout");
        eprintln!("unit_passes stderr");
        assert_eq!(super::answer(), 42);
    }
}
