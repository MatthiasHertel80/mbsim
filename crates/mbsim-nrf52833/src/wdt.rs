//! WDT: watchdog on LFCLK. Once started it cannot be stopped; a timeout generates TIMEOUT and
//! resets the device two LFCLK cycles later (RESETREAS.DOG).

use crate::env::Env;
use mbsim_core::time::TICKS_PER_LFCLK;
use mbsim_core::{Slot, Ticks};

pub const RELOAD_VALUE: u32 = 0x6E52_4635;

#[derive(Debug)]
pub struct Wdt {
    pub slot: Slot,
    pub timeout: bool,
    pub pulses: u32,
    pub inten: u32,
    pub crv: u32,
    pub rren: u32,
    pub config: u32,
    pub running: bool,
    reqstatus: u32,
    started_at: Ticks,
    pub fired: bool,
}

impl Wdt {
    pub fn new(slot: Slot) -> Self {
        Wdt {
            slot,
            timeout: false,
            pulses: 0,
            inten: 0,
            crv: 0xFFFF_FFFF,
            rren: 1,
            config: 1,
            running: false,
            reqstatus: 1,
            started_at: 0,
            fired: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Wdt::new(self.slot);
    }

    pub fn irq(&self) -> bool {
        self.timeout && self.inten & 1 != 0
    }

    fn period(&self) -> Ticks {
        (self.crv as Ticks + 1) * TICKS_PER_LFCLK
    }

    pub fn read(&self, off: u32) -> u32 {
        match off {
            0x100 => self.timeout as u32,
            0x304 | 0x308 => self.inten,
            0x400 => self.running as u32,
            0x404 => self.reqstatus,
            0x504 => self.crv,
            0x508 => self.rren,
            0x50C => self.config,
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32, env: &mut Env) {
        match off {
            0x000 if v & 1 != 0 && !self.running => {
                self.running = true;
                self.started_at = env.now;
                self.reqstatus = self.rren;
                env.schedule(self.slot, env.now + self.period());
            }
            0x100 => self.timeout = v & 1 != 0,
            0x304 => self.inten |= v & 1,
            0x308 => self.inten &= !v,
            0x504 if !self.running => self.crv = v.max(0xF),
            0x508 if !self.running => self.rren = v & 0xFF,
            0x50C if !self.running => self.config = v & 9,
            0x600..=0x61C if self.running && v == RELOAD_VALUE => {
                let n = (off - 0x600) / 4;
                self.reqstatus &= !(1 << n);
                if self.reqstatus & self.rren == 0 {
                    self.reqstatus = self.rren;
                    env.schedule(self.slot, env.now + self.period());
                }
            }
            _ => {}
        }
    }

    /// Timeout or the reset that follows it; returns true when the device must reset.
    pub fn event(&mut self, env: &mut Env) -> bool {
        if self.fired {
            return true;
        }
        self.timeout = true;
        self.pulses |= 1;
        self.fired = true;
        env.schedule(self.slot, env.now + 2 * TICKS_PER_LFCLK);
        false
    }
}
