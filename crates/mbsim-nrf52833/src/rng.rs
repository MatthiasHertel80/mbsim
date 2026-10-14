//! RNG: random number generator. Values come from a deterministic generator so that runs are
//! reproducible; a value takes 30 us (120 us with bias correction).

use crate::env::Env;
use mbsim_core::time::us;
use mbsim_core::Slot;

#[derive(Debug)]
pub struct Rng {
    pub slot: Slot,
    pub valrdy: bool,
    pub pulses: u32,
    pub inten: u32,
    pub shorts: u32,
    pub config: u32,
    pub value: u32,
    running: bool,
    state: u64,
}

impl Rng {
    pub fn new(slot: Slot) -> Self {
        Rng { slot, valrdy: false, pulses: 0, inten: 0, shorts: 0, config: 0, value: 0, running: false, state: 0x853C_49E6_748F_EA9B }
    }

    pub fn reset(&mut self) {
        let state = self.state;
        *self = Rng::new(self.slot);
        self.state = state;
    }

    pub fn irq(&self) -> bool {
        self.valrdy && self.inten & 1 != 0
    }

    fn period(&self) -> mbsim_core::Ticks {
        if self.config & 1 != 0 {
            us(120)
        } else {
            us(30)
        }
    }

    pub fn read(&self, off: u32) -> u32 {
        match off {
            0x100 => self.valrdy as u32,
            0x200 => self.shorts,
            0x304 | 0x308 => self.inten,
            0x504 => self.config,
            0x508 => self.value,
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32, env: &mut Env) {
        match off {
            0x000 if v & 1 != 0 && !self.running => {
                self.running = true;
                env.schedule(self.slot, env.now + self.period());
            }
            0x004 if v & 1 != 0 => {
                self.running = false;
                env.cancel(self.slot);
            }
            0x100 => self.valrdy = v & 1 != 0,
            0x200 => self.shorts = v & 1,
            0x304 => self.inten |= v & 1,
            0x308 => self.inten &= !v,
            0x504 => self.config = v & 1,
            _ => {}
        }
    }

    pub fn event(&mut self, env: &mut Env) {
        if !self.running {
            return;
        }
        // PCG step
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let x = ((self.state >> 18) ^ self.state) >> 27;
        let rot = (self.state >> 59) as u32;
        self.value = ((x as u32).rotate_right(rot)) & 0xFF;
        self.valrdy = true;
        self.pulses |= 1;
        if self.shorts & 1 != 0 {
            self.running = false;
        } else {
            env.schedule(self.slot, env.now + self.period());
        }
    }
}
