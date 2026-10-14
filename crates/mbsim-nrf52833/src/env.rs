//! Context passed to peripheral models.

use crate::board::BoardIo;
use crate::gpio::Gpio;
use mbsim_core::{Scheduler, Slot, Ticks};
use std::collections::HashSet;

/// Scheduler slots of the device.
pub mod slot {
    use mbsim_core::Slot;
    pub const CLOCK_HF: Slot = Slot(0);
    pub const CLOCK_LF: Slot = Slot(1);
    pub const CLOCK_CAL: Slot = Slot(2);
    pub const CLOCK_CT: Slot = Slot(3);
    pub const TIMER: [Slot; 5] = [Slot(4), Slot(5), Slot(6), Slot(7), Slot(8)];
    pub const UARTE0_TX: Slot = Slot(9);
    pub const UARTE0_RX: Slot = Slot(10);
    pub const UARTE0_STOP: Slot = Slot(11);
    pub const UART_HOST: Slot = Slot(12);
    pub const TWIM: [Slot; 2] = [Slot(13), Slot(14)];
    pub const SAADC: Slot = Slot(15);
    pub const PWM: [Slot; 4] = [Slot(16), Slot(17), Slot(18), Slot(19)];
    pub const TEMP: Slot = Slot(20);
    pub const RTC: [Slot; 3] = [Slot(21), Slot(22), Slot(23)];
    pub const NVMC: Slot = Slot(24);
    pub const RNG: Slot = Slot(25);
    pub const WDT: Slot = Slot(26);
    pub const COUNT: usize = 27;
}

/// Warnings about behaviour the model does not cover or that is undefined on the device.
/// Each distinct message is logged once.
#[derive(Default, Debug)]
pub struct Warnings {
    seen: HashSet<String>,
    pub list: Vec<(Ticks, String)>,
}

impl Warnings {
    pub fn warn(&mut self, now: Ticks, msg: impl Into<String>) {
        let msg = msg.into();
        if self.seen.insert(msg.clone()) {
            log::warn!("{msg}");
            self.list.push((now, msg));
        }
    }
}

pub const RAM_BASE: u32 = 0x2000_0000;
pub const RAM_SIZE: u32 = 0x2_0000;

/// What a peripheral can reach while it handles an access or an event.
pub struct Env<'a> {
    pub now: Ticks,
    pub sched: &'a mut Scheduler,
    pub ram: &'a mut [u8],
    pub gpio: &'a mut Gpio,
    pub board: &'a mut dyn BoardIo,
    pub warn: &'a mut Warnings,
}

impl Env<'_> {
    pub fn schedule(&mut self, slot: Slot, at: Ticks) {
        self.sched.schedule(slot, at);
    }

    pub fn cancel(&mut self, slot: Slot) {
        self.sched.cancel(slot);
    }

    /// EasyDMA read of RAM; addresses outside Data RAM read as zero.
    pub fn dma_read(&mut self, addr: u32, buf: &mut [u8], who: &str) {
        for (i, b) in buf.iter_mut().enumerate() {
            let a = addr.wrapping_add(i as u32);
            *b = if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&a) {
                self.ram[(a - RAM_BASE) as usize]
            } else {
                if i == 0 {
                    self.warn.warn(self.now, format!("{who}: EasyDMA read from {a:#010x} outside Data RAM"));
                }
                0
            };
        }
    }

    /// EasyDMA write to RAM; writes outside Data RAM are dropped.
    pub fn dma_write(&mut self, addr: u32, data: &[u8], who: &str) {
        for (i, b) in data.iter().enumerate() {
            let a = addr.wrapping_add(i as u32);
            if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&a) {
                self.ram[(a - RAM_BASE) as usize] = *b;
            } else if i == 0 {
                self.warn.warn(self.now, format!("{who}: EasyDMA write to {a:#010x} outside Data RAM"));
            }
        }
    }
}

/// Register-access helpers for the task/event/interrupt pattern of nRF52 peripherals.
pub fn inten_write(inten: &mut u32, off: u32, v: u32) -> bool {
    match off {
        0x300 => *inten = v,
        0x304 => *inten |= v,
        0x308 => *inten &= !v,
        _ => return false,
    }
    true
}
