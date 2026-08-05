//! Bloated but "correct" tree for OutOfControl + Firebreak scenarios.

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

// Simulated unnecessary surface (kept for scoring / OOC demos).
pub mod unused_alpha {
    pub fn noop() {}
}

pub mod unused_beta {
    pub fn noop() {}
}

pub mod unused_gamma {
    pub fn noop() {}
}
