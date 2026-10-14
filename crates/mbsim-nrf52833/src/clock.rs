//! CLOCK, POWER and APPROTECT, which share the address range of peripheral ID 0.

use crate::env::{inten_write, slot, Env};
use mbsim_core::time::{ms, us};

// INTEN bits of CLOCK and POWER (one shared register)
const HFCLKSTARTED: u32 = 1 << 0;
const LFCLKSTARTED: u32 = 1 << 1;
const DONE: u32 = 1 << 3;
const CTTO: u32 = 1 << 4;
const CTSTARTED: u32 = 1 << 10;
const CTSTOPPED: u32 = 1 << 11;

pub const RESETREAS_RESETPIN: u32 = 1 << 0;
pub const RESETREAS_DOG: u32 = 1 << 1;
pub const RESETREAS_SREQ: u32 = 1 << 2;
pub const RESETREAS_LOCKUP: u32 = 1 << 3;
pub const RESETREAS_OFF: u32 = 1 << 16;
pub const RESETREAS_DIF: u32 = 1 << 18;

#[derive(Debug, Default)]
pub struct ClockPower {
    /// Events (bit positions as INTEN).
    pub events: u32,
    /// Events generated since the last collection (bit n = EVENTS at 0x100 + 4n).
    pub pulses: u32,
    pub inten: u32,
    pub hfxo_running: bool,
    pub hf_requested: bool,
    pub lf_running: bool,
    pub lf_src: u32,
    pub lf_src_copy: u32,
    pub lf_requested: bool,
    pub ctiv: u32,
    pub traceconfig: u32,
    pub hfxodebounce: u32,
    pub lfrcmode: u32,
    // POWER
    pub resetreas: u32,
    pub pofcon: u32,
    pub gpregret: [u32; 2],
    pub dcdcen: u32,
    pub dcdcen0: u32,
    pub ram_power: [u32; 9],
    pub system_off: bool,
    // APPROTECT
    pub approtect_disable: u32,
    pub approtect_force: u32,
}

impl ClockPower {
    /// Generates events: sets the event bits and records them as pulses for PPI.
    #[inline]
    fn ev(&mut self, bits: u32) {
        self.events |= bits;
        self.pulses |= bits;
    }

    pub fn new() -> Self {
        ClockPower { ram_power: [0xFFFF; 9], hfxodebounce: 0x10, ..Default::default() }
    }

    /// Reset; RESETREAS, GPREGRET and RAM retention survive a system reset.
    pub fn reset(&mut self, keep_retained: bool) {
        let (resetreas, gpregret) = (self.resetreas, self.gpregret);
        *self = ClockPower::new();
        if keep_retained {
            self.resetreas = resetreas;
            self.gpregret = gpregret;
        }
    }

    pub fn irq(&self) -> bool {
        self.events & self.inten != 0
    }

    fn event_reg(off: u32) -> Option<u32> {
        Some(match off {
            0x100 => HFCLKSTARTED,
            0x104 => LFCLKSTARTED,
            0x108 => 1 << 2, // POFWARN
            0x10C => DONE,
            0x110 => CTTO,
            0x114 => 1 << 5, // SLEEPENTER
            0x118 => 1 << 6, // SLEEPEXIT
            0x11C => 1 << 7, // USBDETECTED
            0x120 => 1 << 8, // USBREMOVED
            0x124 => 1 << 9, // USBPWRRDY
            0x128 => CTSTARTED,
            0x12C => CTSTOPPED,
            _ => return None,
        })
    }

    pub fn read(&mut self, off: u32) -> u32 {
        if let Some(bit) = Self::event_reg(off) {
            return (self.events & bit != 0) as u32;
        }
        match off {
            0x304 | 0x308 => self.inten,
            0x400 => self.resetreas,
            0x408 => self.hf_requested as u32,
            0x40C => (self.hfxo_running as u32) | ((self.hfxo_running as u32) << 16),
            0x414 => self.lf_requested as u32,
            0x418 => self.lf_src_copy | ((self.lf_running as u32) << 16),
            0x41C => self.lf_src_copy,
            0x428 => 0x3, // RAMSTATUS
            0x438 => 0,   // USBREGSTATUS: VBUS not present on the target (USB goes to the interface MCU)
            0x510 => self.pofcon,
            0x518 => self.lf_src,
            0x51C => self.gpregret[0],
            0x520 => self.gpregret[1],
            0x528 => self.hfxodebounce,
            0x538 => self.ctiv,
            0x550 => self.approtect_force,
            0x558 => self.approtect_disable,
            0x55C => self.traceconfig,
            0x578 => self.dcdcen,
            0x580 => self.dcdcen0,
            0x5B4 => self.lfrcmode,
            0x640 => 0, // MAINREGSTATUS: normal voltage
            0x900..=0x98C => {
                let i = ((off - 0x900) / 16) as usize;
                if (off - 0x900).is_multiple_of(16) {
                    self.ram_power[i]
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32, env: &mut Env) {
        if let Some(bit) = Self::event_reg(off) {
            if v & 1 == 0 {
                self.events &= !bit;
            } else {
                self.events |= bit;
            }
            return;
        }
        if inten_write(&mut self.inten, off, v) {
            self.inten &= 0xFFF;
            return;
        }
        let now = env.now;
        match off {
            0x000 if v & 1 != 0 => {
                // HFCLKSTART: crystal oscillator startup (tXO,START 0.36 ms typ.)
                self.hf_requested = true;
                if !self.hfxo_running {
                    env.schedule(slot::CLOCK_HF, now + us(360));
                } else {
                    env.schedule(slot::CLOCK_HF, now + us(1));
                }
            }
            0x004 if v & 1 != 0 => {
                self.hf_requested = false;
                self.hfxo_running = false;
                env.cancel(slot::CLOCK_HF);
            }
            0x008 if v & 1 != 0 => {
                self.lf_requested = true;
                self.lf_src_copy = self.lf_src & 3;
                let t = match self.lf_src_copy {
                    1 => ms(250), // LFXO (not fitted on the micro:bit)
                    _ => us(600),
                };
                env.schedule(slot::CLOCK_LF, now + t);
            }
            0x00C if v & 1 != 0 => {
                self.lf_requested = false;
                self.lf_running = false;
                env.cancel(slot::CLOCK_LF);
            }
            0x010 if v & 1 != 0 => env.schedule(slot::CLOCK_CAL, now + ms(16)),
            0x014 if v & 1 != 0 => {
                self.ev(CTSTARTED);
                if self.ctiv > 0 {
                    env.schedule(slot::CLOCK_CT, now + ms(250) * self.ctiv as u64);
                }
            }
            0x018 if v & 1 != 0 => {
                env.cancel(slot::CLOCK_CT);
                self.ev(CTSTOPPED);
            }
            0x400 => self.resetreas &= !v,
            0x500 if v & 1 != 0 => {
                self.system_off = true;
                env.warn.warn(now, "POWER: SYSTEMOFF entered; the board needs a reset to wake up");
            }
            0x510 => self.pofcon = v & 0xF1F,
            0x518 => self.lf_src = v & 0x3_0003,
            0x51C => self.gpregret[0] = v & 0xFF,
            0x520 => self.gpregret[1] = v & 0xFF,
            0x528 => self.hfxodebounce = v & 0xFF,
            0x538 => self.ctiv = v & 0x7F,
            0x550 if v == 0 => {
                self.approtect_force = 0;
            }
            0x558 => self.approtect_disable = v & 0xFF,
            0x55C => self.traceconfig = v & 0x3_0003,
            0x578 => self.dcdcen = v & 1,
            0x580 => self.dcdcen0 = v & 1,
            0x5B4 => self.lfrcmode = v & 1,
            0x900..=0x98C => {
                let i = ((off - 0x900) / 16) as usize;
                match (off - 0x900) % 16 {
                    0 => self.ram_power[i] = v,
                    4 => self.ram_power[i] |= v,
                    8 => self.ram_power[i] &= !v,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub fn event(&mut self, s: mbsim_core::Slot) {
        match s {
            slot::CLOCK_HF => {
                self.hfxo_running = true;
                self.ev(HFCLKSTARTED);
            }
            slot::CLOCK_LF => {
                self.lf_running = true;
                self.ev(LFCLKSTARTED);
            }
            slot::CLOCK_CAL => self.ev(DONE),
            slot::CLOCK_CT => self.ev(CTTO),
            _ => {}
        }
    }
}
