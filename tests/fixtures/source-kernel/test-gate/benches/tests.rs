//! Reached by a bench target through a gated declaration. Cargo already calls
//! the target non-production, so the gate changes nothing here.

pub fn benched() -> usize {
    9
}
