//! TIMER0..TIMER4. The counter is computed from the time of the last (re)start, so a running
//! timer costs one scheduled event per compare match.

use crate::env::{inten_write, Env};
use mbsim_core::time::TICKS_PER_16MHZ;
use mbsim_core::{Slot, Ticks};

#[derive(Debug)]
pub struct Timer {
    pub id: usize,
    pub slot: Slot,
    pub ncc: usize,
    pub events: u32,
    /// Events generated since the last collection (bit n = EVENTS at 0x100 + 4n).
    pub pulses: u32,
    pub inten: u32,
    pub shorts: u32,
    pub mode: u32,
    pub bitmode: u32,
    pub prescaler: u32,
    pub cc: [u32; 6],
    pub running: bool,
    /// Counter value at `base_time` (timer mode) or the current value (counter mode).
    base: u32,
    base_time: Ticks,
    /// Next compare match time per CC (timer mode).
    next: [Option<Ticks>; 6],
}

impl Timer {
    pub fn new(id: usize, slot: Slot) -> Self {
        Timer {
            id,
            slot,
            ncc: if id >= 3 { 6 } else { 4 },
            events: 0,
            pulses: 0,
            inten: 0,
            shorts: 0,
            mode: 0,
            bitmode: 0,
            prescaler: 4,
            cc: [0; 6],
            running: false,
            base: 0,
            base_time: 0,
            next: [None; 6],
        }
    }

    pub fn reset(&mut self) {
        *self = Timer::new(self.id, self.slot);
    }

    pub fn irq(&self) -> bool {
        (self.events << 16) & self.inten != 0
    }

    fn mask(&self) -> u32 {
        match self.bitmode & 3 {
            0 => 0xFFFF,
            1 => 0xFF,
            2 => 0xFF_FFFF,
            _ => 0xFFFF_FFFF,
        }
    }

    fn period(&self) -> Ticks {
        TICKS_PER_16MHZ << self.prescaler.min(9)
    }

    fn timer_mode(&self) -> bool {
        self.mode & 3 == 0
    }

    /// Current counter value.
    pub fn counter(&self, now: Ticks) -> u32 {
        if self.running && self.timer_mode() {
            let k = (now - self.base_time) / self.period();
            (self.base as u64 + k) as u32 & self.mask()
        } else {
            self.base & self.mask()
        }
    }

    fn rebase(&mut self, now: Ticks) {
        let c = self.counter(now);
        self.base = c;
        if self.running && self.timer_mode() {
            // keep the phase of the counter clock
            let k = (now - self.base_time) / self.period();
            self.base_time += k * self.period();
        } else {
            self.base_time = now;
        }
    }

    fn reschedule(&mut self, env: &mut Env) {
        self.next = [None; 6];
        if !self.running || !self.timer_mode() {
            env.cancel(self.slot);
            return;
        }
        let now = env.now;
        let period = self.period();
        let k_now = (now - self.base_time) / period;
        let cur = (self.base as u64 + k_now) as u32 & self.mask();
        let wrap = self.mask() as u64 + 1;
        let mut earliest = Ticks::MAX;
        for i in 0..self.ncc {
            let cc = self.cc[i] & self.mask();
            let mut delta = (cc.wrapping_sub(cur) & self.mask()) as u64;
            if delta == 0 {
                delta = wrap;
            }
            let t = self.base_time + (k_now + delta) * period;
            self.next[i] = Some(t);
            earliest = earliest.min(t);
        }
        env.schedule(self.slot, earliest);
    }

    fn compare_hit(&mut self, i: usize, env: &mut Env) {
        self.events |= 1 << i;
        self.pulses |= 1 << (16 + i);
        let now = env.now;
        if self.shorts & (1 << i) != 0 {
            // COMPAREn_CLEAR
            self.base = 0;
            self.base_time = now;
        }
        if self.shorts & (1 << (i + 8)) != 0 {
            // COMPAREn_STOP
            self.rebase(now);
            self.running = false;
        }
    }

    pub fn read(&mut self, off: u32) -> u32 {
        match off {
            0x140..=0x154 => {
                let i = (off - 0x140) / 4;
                (self.events >> i) & 1
            }
            0x200 => self.shorts,
            0x304 | 0x308 => self.inten,
            0x504 => self.mode,
            0x508 => self.bitmode,
            0x510 => self.prescaler,
            0x540..=0x554 => {
                let i = ((off - 0x540) / 4) as usize;
                if i < self.ncc {
                    self.cc[i]
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32, env: &mut Env) {
        let now = env.now;
        match off {
            0x000 if v & 1 != 0 && !self.running => {
                self.base_time = now;
                self.running = true;
                self.reschedule(env);
            }
            0x004 if v & 1 != 0 && self.running => {
                self.rebase(now);
                self.running = false;
                self.reschedule(env);
            }
            0x008 if v & 1 != 0 && self.running && !self.timer_mode() => {
                self.base = (self.base + 1) & self.mask();
                for i in 0..self.ncc {
                    if self.base == self.cc[i] & self.mask() {
                        self.compare_hit(i, env);
                    }
                }
            }
            0x00C if v & 1 != 0 => {
                self.base = 0;
                self.base_time = now;
                self.reschedule(env);
            }
            0x010 if v & 1 != 0 => {
                // SHUTDOWN
                self.running = false;
                self.base = 0;
                self.reschedule(env);
            }
            0x040..=0x054 if v & 1 != 0 => {
                let i = ((off - 0x040) / 4) as usize;
                if i < self.ncc {
                    self.cc[i] = self.counter(now);
                    self.reschedule(env);
                }
            }
            0x140..=0x154 => {
                let i = (off - 0x140) / 4;
                if v & 1 == 0 {
                    self.events &= !(1 << i);
                } else {
                    self.events |= 1 << i;
                }
            }
            0x200 => self.shorts = v & 0x3F3F,
            0x300..=0x308 => {
                inten_write(&mut self.inten, off, v);
                self.inten &= 0x3F << 16;
            }
            0x504 => {
                self.rebase(now);
                self.mode = v & 3;
                self.reschedule(env);
            }
            0x508 => {
                self.rebase(now);
                self.bitmode = v & 3;
                self.reschedule(env);
            }
            0x510 => {
                self.rebase(now);
                self.prescaler = v & 0xF;
                self.base_time = now;
                self.reschedule(env);
            }
            0x540..=0x554 => {
                let i = ((off - 0x540) / 4) as usize;
                if i < self.ncc {
                    self.cc[i] = v;
                    self.reschedule(env);
                }
            }
            _ => {}
        }
    }

    pub fn event(&mut self, env: &mut Env) {
        let now = env.now;
        for i in 0..self.ncc {
            if self.next[i].is_some_and(|t| t <= now) {
                self.compare_hit(i, env);
            }
        }
        self.reschedule(env);
    }
}
