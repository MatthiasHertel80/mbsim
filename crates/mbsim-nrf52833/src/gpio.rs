//! GPIO P0 (32 pins) and P1 (10 pins).
//!
//! The pin state the device drives is computed from PIN_CNF, OUT and the peripherals that take
//! over a pin (PWM, UARTE TXD, TWIM SCL/SDA). Every change is reported to the board; inputs are
//! read from the board.

use crate::board::{BoardIo, PinDrive, NUM_PINS};
use mbsim_core::Ticks;

pub const P0_BASE: u32 = 0x5000_0000;
pub const P1_BASE: u32 = 0x5000_0300;

#[derive(Clone, Debug)]
pub struct Port {
    pub out: u32,
    pub cnf: [u32; 32],
    pub latch: u32,
    pub detectmode: u32,
    pub pins: u32,
}

impl Port {
    fn new(pins: u32) -> Self {
        Port { out: 0, cnf: [2; 32], latch: 0, detectmode: 0, pins }
    }

    fn mask(&self) -> u32 {
        if self.pins >= 32 {
            u32::MAX
        } else {
            (1 << self.pins) - 1
        }
    }

    fn dir(&self) -> u32 {
        (0..32).fold(0, |d, i| d | ((self.cnf[i] & 1) << i))
    }
}

/// A peripheral that controls a pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinOverride {
    pub level: bool,
    /// The peripheral also enables the output buffer.
    pub force_output: bool,
    /// The peripheral uses the pin as an input (GPIOTE event mode): the output is disabled.
    pub force_input: bool,
}

impl PinOverride {
    pub const fn output(level: bool) -> Self {
        PinOverride { level, force_output: true, force_input: false }
    }

    pub const fn value(level: bool) -> Self {
        PinOverride { level, force_output: false, force_input: false }
    }

    pub const fn input() -> Self {
        PinOverride { level: false, force_output: false, force_input: true }
    }
}

pub struct Gpio {
    pub ports: [Port; 2],
    overrides: [Option<PinOverride>; NUM_PINS],
    last: [PinDrive; NUM_PINS],
}

impl Default for Gpio {
    fn default() -> Self {
        Self::new()
    }
}

/// Pin number (32 * port + pin) of a PSEL register value, if connected.
pub fn psel_pin(v: u32) -> Option<u8> {
    if v & 0x8000_0000 != 0 {
        return None;
    }
    let pin = (v & 0x3F) as usize;
    if pin < NUM_PINS {
        Some(pin as u8)
    } else {
        None
    }
}

impl Gpio {
    pub fn new() -> Self {
        Gpio { ports: [Port::new(32), Port::new(10)], overrides: [None; NUM_PINS], last: [PinDrive::default(); NUM_PINS] }
    }

    pub fn reset(&mut self, now: Ticks, board: &mut dyn BoardIo) {
        self.ports = [Port::new(32), Port::new(10)];
        self.overrides = [None; NUM_PINS];
        for p in 0..NUM_PINS as u8 {
            self.update_pin(p, now, board);
        }
    }

    /// The state the device currently drives on a pin.
    pub fn drive(&self, pin: u8) -> PinDrive {
        let port = &self.ports[(pin >> 5) as usize];
        let n = (pin & 31) as usize;
        let cnf = port.cnf[n];
        let ov = self.overrides[pin as usize];
        PinDrive {
            output: (cnf & 1 != 0 || ov.is_some_and(|o| o.force_output)) && !ov.is_some_and(|o| o.force_input),
            level: ov.map(|o| o.level).unwrap_or(port.out >> n & 1 != 0),
            drive: ((cnf >> 8) & 7) as u8,
            input: cnf & 2 == 0 || ov.is_some_and(|o| o.force_input),
            pull: ((cnf >> 2) & 3) as u8,
        }
    }

    pub fn update_pin(&mut self, pin: u8, now: Ticks, board: &mut dyn BoardIo) {
        let d = self.drive(pin);
        if d != self.last[pin as usize] {
            self.last[pin as usize] = d;
            board.pin_drive(now, pin, d);
        }
    }

    fn update_port(&mut self, port: usize, mask: u32, now: Ticks, board: &mut dyn BoardIo) {
        let pins = self.ports[port].pins;
        for n in 0..pins {
            if mask & (1 << n) != 0 {
                self.update_pin((port as u8) << 5 | n as u8, now, board);
            }
        }
    }

    /// A peripheral takes over (`Some`) or releases (`None`) a pin.
    pub fn set_override(&mut self, pin: u8, ov: Option<PinOverride>, now: Ticks, board: &mut dyn BoardIo) {
        if (pin as usize) < NUM_PINS && self.overrides[pin as usize] != ov {
            self.overrides[pin as usize] = ov;
            self.update_pin(pin, now, board);
        }
    }

    /// Reports a future edge of a peripheral output (PWM) to the board without changing the
    /// current state.
    pub fn report_edge(&mut self, pin: u8, at: Ticks, level: bool, board: &mut dyn BoardIo) {
        if (pin as usize) < NUM_PINS {
            let mut d = self.drive(pin);
            d.level = level;
            if d != self.last[pin as usize] {
                self.last[pin as usize] = d;
                board.pin_drive(at, pin, d);
            }
        }
    }

    fn input(&self, port: usize, now: Ticks, board: &mut dyn BoardIo) -> u32 {
        let mut v = 0;
        for n in 0..self.ports[port].pins {
            let pin = ((port as u8) << 5) | n as u8;
            let d = self.drive(pin);
            if d.input && board.pin_input(now, pin, d) {
                v |= 1 << n;
            }
        }
        v
    }

    fn update_latch(&mut self, port: usize, now: Ticks, board: &mut dyn BoardIo) {
        let input = self.input(port, now, board);
        let p = &mut self.ports[port];
        for n in 0..p.pins as usize {
            let sense = (p.cnf[n] >> 16) & 3;
            let hit = match sense {
                2 => input >> n & 1 == 1,
                3 => input >> n & 1 == 0,
                _ => false,
            };
            if hit {
                p.latch |= 1 << n;
            }
        }
    }

    /// The DETECT signal: a pin whose SENSE condition holds (or LATCH set in LDETECT mode).
    pub fn detect(&mut self, now: Ticks, board: &mut dyn BoardIo) -> bool {
        let mut any = false;
        for port in 0..2 {
            let sensing = (0..self.ports[port].pins as usize).any(|n| (self.ports[port].cnf[n] >> 16) & 3 >= 2);
            if !sensing {
                continue;
            }
            self.update_latch(port, now, board);
            let p = &self.ports[port];
            if p.detectmode & 1 != 0 {
                any |= p.latch != 0;
            } else {
                let input = self.input(port, now, board);
                for n in 0..p.pins as usize {
                    match (p.cnf[n] >> 16) & 3 {
                        2 => any |= input >> n & 1 == 1,
                        3 => any |= input >> n & 1 == 0,
                        _ => {}
                    }
                }
            }
        }
        any
    }

    /// True when any pin has a SENSE condition configured.
    pub fn sensing(&self) -> bool {
        self.ports.iter().any(|p| (0..p.pins as usize).any(|n| (p.cnf[n] >> 16) & 3 >= 2))
    }

    /// Register read; `off` is relative to the port base.
    pub fn read(&mut self, port: usize, off: u32, now: Ticks, board: &mut dyn BoardIo) -> u32 {
        let mask = self.ports[port].mask();
        match off {
            0x504 | 0x508 | 0x50C => self.ports[port].out & mask,
            0x510 => self.input(port, now, board),
            0x514 | 0x518 | 0x51C => self.ports[port].dir() & mask,
            0x520 => {
                self.update_latch(port, now, board);
                self.ports[port].latch & mask
            }
            0x524 => self.ports[port].detectmode,
            0x700..=0x77C => {
                let n = ((off - 0x700) / 4) as usize;
                if (n as u32) < self.ports[port].pins {
                    self.ports[port].cnf[n]
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, port: usize, off: u32, v: u32, now: Ticks, board: &mut dyn BoardIo) {
        let mask = self.ports[port].mask();
        let p = &mut self.ports[port];
        let changed = match off {
            0x504 => {
                let c = (p.out ^ v) & mask;
                p.out = v & mask;
                c
            }
            0x508 => {
                let c = v & !p.out & mask;
                p.out |= v & mask;
                c
            }
            0x50C => {
                let c = v & p.out & mask;
                p.out &= !v;
                c
            }
            0x514 | 0x518 | 0x51C => {
                let old = p.dir();
                let new = match off {
                    0x514 => v,
                    0x518 => old | v,
                    _ => old & !v,
                } & mask;
                for n in 0..32 {
                    p.cnf[n] = (p.cnf[n] & !1) | ((new >> n) & 1);
                }
                old ^ new
            }
            0x520 => {
                p.latch &= !v;
                0
            }
            0x524 => {
                p.detectmode = v & 1;
                0
            }
            0x700..=0x77C => {
                let n = ((off - 0x700) / 4) as usize;
                if (n as u32) < p.pins {
                    p.cnf[n] = v & 0x0003_070F;
                    1 << n
                } else {
                    0
                }
            }
            _ => 0,
        };
        if changed != 0 {
            self.update_port(port, changed, now, board);
        }
    }
}
