//! `web_time` API on top of `std::time`, with an instruction-counter `Instant` on wasm32.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::*;

#[cfg(target_arch = "wasm32")]
pub use std::time::{Duration, SystemTime, SystemTimeError, TryFromFloatSecsError, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(u64);

#[cfg(target_arch = "wasm32")]
impl Instant {
    /// Call-context instruction counter (type 1): monotonic across awaits.
    pub fn now() -> Self {
        Self(ic0::performance_counter(1))
    }
    pub fn duration_since(&self, earlier: Self) -> Duration {
        self.saturating_duration_since(earlier)
    }
    pub fn checked_duration_since(&self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0).map(Duration::from_nanos)
    }
    pub fn saturating_duration_since(&self, earlier: Self) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
    pub fn elapsed(&self) -> Duration {
        Self::now().saturating_duration_since(*self)
    }
    pub fn checked_add(&self, d: Duration) -> Option<Self> {
        u64::try_from(d.as_nanos()).ok().and_then(|n| self.0.checked_add(n)).map(Self)
    }
    pub fn checked_sub(&self, d: Duration) -> Option<Self> {
        u64::try_from(d.as_nanos()).ok().and_then(|n| self.0.checked_sub(n)).map(Self)
    }
}

#[cfg(target_arch = "wasm32")]
impl std::ops::Add<Duration> for Instant {
    type Output = Instant;
    fn add(self, d: Duration) -> Instant {
        self.checked_add(d).expect("overflow when adding duration to instant")
    }
}

#[cfg(target_arch = "wasm32")]
impl std::ops::Sub<Instant> for Instant {
    type Output = Duration;
    fn sub(self, other: Instant) -> Duration {
        self.duration_since(other)
    }
}
