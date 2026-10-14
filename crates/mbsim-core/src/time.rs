//! Simulation time.
//!
//! One tick is 1/512 MHz (1.953125 ns), the least common multiple of the 64 MHz CPU clock and the
//! 32.768 kHz low-frequency clock, so that every clock of the nRF52833 has a whole number of ticks
//! per period. A `u64` covers more than 1000 years.

/// Simulation time in ticks of 1/512 MHz.
pub type Ticks = u64;

pub const TICKS_PER_SEC: Ticks = 512_000_000;
pub const TICKS_PER_MS: Ticks = 512_000;
pub const TICKS_PER_US: Ticks = 512;
/// 64 MHz CPU clock (HCLK).
pub const TICKS_PER_CPU_CYCLE: Ticks = 8;
/// 16 MHz peripheral clock (PCLK16M).
pub const TICKS_PER_16MHZ: Ticks = 32;
/// 1 MHz peripheral clock (PCLK1M).
pub const TICKS_PER_1MHZ: Ticks = 512;
/// 32.768 kHz low-frequency clock (LFCLK).
pub const TICKS_PER_LFCLK: Ticks = 15625;

pub const fn us(n: u64) -> Ticks {
    n * TICKS_PER_US
}

pub const fn ms(n: u64) -> Ticks {
    n * TICKS_PER_MS
}

/// Rounds a time up to the next CPU clock edge.
pub const fn ceil_cpu_cycle(t: Ticks) -> Ticks {
    t.div_ceil(TICKS_PER_CPU_CYCLE) * TICKS_PER_CPU_CYCLE
}

/// Converts ticks to seconds (for display only).
pub fn to_secs(t: Ticks) -> f64 {
    t as f64 / TICKS_PER_SEC as f64
}

/// Converts seconds to ticks, rounding to the nearest tick.
pub fn from_secs(s: f64) -> Ticks {
    (s * TICKS_PER_SEC as f64).round().max(0.0) as Ticks
}

/// Accumulates a period that is not a whole number of ticks (baud and I2C bit times), so that the
/// error of a sequence of periods stays below one tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct FracPeriod {
    /// Period in units of 1/2^32 ticks.
    pub period_q32: u128,
}

impl FracPeriod {
    /// A period of `ticks_num / ticks_den` ticks.
    pub fn new(ticks_num: u128, ticks_den: u128) -> Self {
        Self { period_q32: (ticks_num << 32) / ticks_den.max(1) }
    }

    /// Time of the end of the `n`-th period after `start`.
    pub fn after(&self, start: Ticks, n: u64) -> Ticks {
        start + ((self.period_q32 * n as u128 + (1 << 31)) >> 32) as Ticks
    }
}
