//! The interface between the device and the board around it.

use mbsim_core::Ticks;

/// Number of GPIO pins (P0.00..P0.31, P1.00..P1.09).
pub const NUM_PINS: usize = 42;

/// The state the device drives onto a pin, as configured by GPIO and the peripherals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PinDrive {
    /// Output buffer enabled (DIR = output).
    pub output: bool,
    /// Output level.
    pub level: bool,
    /// PIN_CNF.DRIVE (0 S0S1 .. 7 H0D1).
    pub drive: u8,
    /// Input buffer connected.
    pub input: bool,
    /// PIN_CNF.PULL: 0 none, 1 pull-down, 3 pull-up.
    pub pull: u8,
}

impl PinDrive {
    /// The driven level, or `None` when the output is high impedance.
    pub fn driven(&self) -> Option<bool> {
        if !self.output {
            return None;
        }
        match (self.level, self.drive) {
            (false, 4 | 5) => None,
            (true, 6 | 7) => None,
            (l, _) => Some(l),
        }
    }

    /// True when the driven level uses the high-drive strength.
    pub fn high_drive(&self) -> bool {
        matches!((self.level, self.drive), (false, 1 | 3 | 7) | (true, 2 | 3 | 5))
    }

    pub fn pull_up(&self) -> bool {
        self.pull == 3
    }

    pub fn pull_down(&self) -> bool {
        self.pull == 1
    }
}

/// Result of an I2C transfer phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I2cResult {
    Ack,
    /// No device acknowledged the address.
    AddrNack,
    /// The device did not acknowledge data byte `n`.
    DataNack(usize),
}

/// What the board provides to the device.
pub trait BoardIo {
    /// The device drives a pin differently (called for every change, at time `now`; for PWM
    /// output also for edges slightly in the future, in order).
    fn pin_drive(&mut self, now: Ticks, pin: u8, drive: PinDrive);

    /// Level seen by the input buffer of a pin.
    fn pin_input(&mut self, now: Ticks, pin: u8, drive: PinDrive) -> bool;

    /// Voltage at an analog-capable pin in volts.
    fn analog_input(&mut self, now: Ticks, pin: u8) -> f64;

    /// Supply voltage.
    fn vdd(&self) -> f64 {
        3.3
    }

    /// I2C write transaction (address with W, then `data`) on the bus with these pins.
    fn i2c_write(&mut self, now: Ticks, scl: u8, sda: u8, addr: u8, data: &[u8]) -> I2cResult;

    /// I2C read transaction (address with R, then `buf.len()` bytes).
    fn i2c_read(&mut self, now: Ticks, scl: u8, sda: u8, addr: u8, buf: &mut [u8]) -> I2cResult;

    /// A UART frame with `byte` completed on pin `pin`.
    fn uart_tx(&mut self, now: Ticks, pin: u8, byte: u8);

    /// Die temperature in degrees Celsius.
    fn temperature(&mut self, now: Ticks) -> f64;

    /// Next time the board changes an input by itself (sensor interrupts); `Ticks::MAX` if never.
    fn next_event(&mut self, _now: Ticks) -> Ticks {
        Ticks::MAX
    }
}

/// A board with nothing connected (for tests).
#[derive(Default)]
pub struct NullBoard {
    pub uart: Vec<u8>,
}

impl BoardIo for NullBoard {
    fn pin_drive(&mut self, _now: Ticks, _pin: u8, _drive: PinDrive) {}

    fn pin_input(&mut self, _now: Ticks, _pin: u8, d: PinDrive) -> bool {
        d.driven().unwrap_or(d.pull_up())
    }

    fn analog_input(&mut self, _now: Ticks, _pin: u8) -> f64 {
        0.0
    }

    fn i2c_write(&mut self, _now: Ticks, _scl: u8, _sda: u8, _addr: u8, _data: &[u8]) -> I2cResult {
        I2cResult::AddrNack
    }

    fn i2c_read(&mut self, _now: Ticks, _scl: u8, _sda: u8, _addr: u8, _buf: &mut [u8]) -> I2cResult {
        I2cResult::AddrNack
    }

    fn uart_tx(&mut self, _now: Ticks, _pin: u8, byte: u8) {
        self.uart.push(byte);
    }

    fn temperature(&mut self, _now: Ticks) -> f64 {
        21.0
    }
}
