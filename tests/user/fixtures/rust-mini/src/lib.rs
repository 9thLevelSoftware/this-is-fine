//! Minimal fixture for AI user-testing (Tier A/B/C).

/// Trivial pure function used as a "small bug already solvable" baseline.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_works() {
        assert_eq!(add(2, 2), 4);
    }
}
