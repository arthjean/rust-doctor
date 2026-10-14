//! Trigger and silence of `clippy::missing_transmute_annotations`.
//!
//! Both functions perform the same transmute into a binding. The positive
//! lets inference pick both types, so a change to the binding's use could
//! change what is reinterpreted; the negative names them.

/// A length, in meters, with the layout of the `u64` it wraps.
#[repr(transparent)]
pub struct Meters(pub u64);

/// Wraps a raw length.
pub fn positive(raw: u64) -> Meters {
    // SAFETY: `Meters` is a transparent wrapper around `u64`.
    let meters = unsafe { std::mem::transmute(raw) };
    meters
}

/// Wraps a raw length.
pub fn negative(raw: u64) -> Meters {
    // SAFETY: `Meters` is a transparent wrapper around `u64`.
    let meters = unsafe { std::mem::transmute::<u64, Meters>(raw) };
    meters
}
