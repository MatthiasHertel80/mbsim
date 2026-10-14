//! TEMP: die temperature sensor (conversion time 36 us, result in 0.25 degree units).

use crate::env::Env;
use mbsim_core::time::us;
use mbsim_core::Slot;

#[derive(Debug)]
pub struct Temp {
    pub slot: Slot,
    pub datardy: bool,
    pub pulses: u32,
    pub inten: u32,
    pub temp: i32,
    pub a: [u32; 6],
    pub b: [u32; 6],
    pub t: [u32; 5],
}

impl Temp {
    pub fn new(slot: Slot) -> Self {
        Temp {
            slot,
            datardy: false,
            pulses: 0,
            inten: 0,
            temp: 0,
            a: [0x326, 0x348, 0x3AA, 0x40E, 0x4BD, 0x5A3],
            b: [0x3FCC, 0x3F98, 0x3F98, 0x0012, 0x004D, 0x3E10],
            t: [0xE2, 0x19, 0x3C, 0x50, 0x6E],
        }
    }

    pub fn reset(&mut self) {
        *self = Temp::new(self.slot);
    }

    pub fn irq(&self) -> bool {
        self.datardy && self.inten & 1 != 0
    }

    pub fn read(&mut self, off: u32) -> u32 {
        match off {
            0x100 => self.datardy as u32,
            0x304 | 0x308 => self.inten,
            0x508 => self.temp as u32,
            0x520..=0x534 => self.a[((off - 0x520) / 4) as usize],
            0x540..=0x554 => self.b[((off - 0x540) / 4) as usize],
            0x560..=0x570 => self.t[((off - 0x560) / 4) as usize],
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32, env: &mut Env) {
        match off {
            0x000 if v & 1 != 0 => env.schedule(self.slot, env.now + us(36)),
            0x004 if v & 1 != 0 => env.cancel(self.slot),
            0x100 => self.datardy = v & 1 != 0,
            0x304 => self.inten |= v & 1,
            0x308 => self.inten &= !v,
            0x520..=0x534 => self.a[((off - 0x520) / 4) as usize] = v & 0xFFF,
            0x540..=0x554 => self.b[((off - 0x540) / 4) as usize] = v & 0x3FFF,
            0x560..=0x570 => self.t[((off - 0x560) / 4) as usize] = v & 0xFF,
            _ => {}
        }
    }

    pub fn event(&mut self, env: &mut Env) {
        let c = env.board.temperature(env.now);
        self.temp = (c * 4.0).round() as i32;
        self.datardy = true;
        self.pulses |= 1;
    }
}
