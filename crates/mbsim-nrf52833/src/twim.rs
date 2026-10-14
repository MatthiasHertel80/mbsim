//! TWIM (I2C master with EasyDMA), modelled at transaction level with bus timing.
//!
//! A write transfers all TXD bytes to the device on the bus selected by PSEL.SCL/SDA when the
//! transaction starts; the events follow with the bit timing of FREQUENCY.

use crate::board::I2cResult;
use crate::env::{inten_write, Env};
use crate::gpio::psel_pin;
use mbsim_core::time::{FracPeriod, TICKS_PER_SEC};
use mbsim_core::{Slot, Ticks};
use std::collections::VecDeque;

const STOPPED: u32 = 1 << 1;
const ERROR: u32 = 1 << 9;
const SUSPENDED: u32 = 1 << 18;
const RXSTARTED: u32 = 1 << 19;
const TXSTARTED: u32 = 1 << 20;
const LASTRX: u32 = 1 << 23;
const LASTTX: u32 = 1 << 24;

const SHORT_LASTTX_STARTRX: u32 = 1 << 7;
const SHORT_LASTTX_SUSPEND: u32 = 1 << 8;
const SHORT_LASTTX_STOP: u32 = 1 << 9;
const SHORT_LASTRX_STARTTX: u32 = 1 << 10;
const SHORT_LASTRX_STOP: u32 = 1 << 12;

fn event_bit(off: u32) -> Option<u32> {
    Some(match off {
        0x104 => STOPPED,
        0x124 => ERROR,
        0x148 => SUSPENDED,
        0x14C => RXSTARTED,
        0x150 => TXSTARTED,
        0x15C => LASTRX,
        0x160 => LASTTX,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Event(u32),
    TxDone,
    RxDone,
    Stopped,
}

#[derive(Debug)]
pub struct Twim {
    pub name: &'static str,
    pub slot: Slot,
    pub events: u32,
    /// Events generated since the last collection (bit n = EVENTS at 0x100 + 4n).
    pub pulses: u32,
    pub inten: u32,
    pub shorts: u32,
    pub errorsrc: u32,
    pub enable: u32,
    pub psel_scl: u32,
    pub psel_sda: u32,
    pub frequency: u32,
    pub rxd_ptr: u32,
    pub rxd_maxcnt: u32,
    pub rxd_amount: u32,
    pub rxd_list: u32,
    pub txd_ptr: u32,
    pub txd_maxcnt: u32,
    pub txd_amount: u32,
    pub txd_list: u32,
    pub address: u32,
    queue: VecDeque<(Ticks, Action)>,
    busy: bool,
    rx_data: Vec<u8>,
    rx_ptr: u32,
}

impl Twim {
    /// Generates events: sets the event bits and records them as pulses for PPI.
    #[inline]
    fn ev(&mut self, bits: u32) {
        self.events |= bits;
        self.pulses |= bits;
    }

    pub fn new(name: &'static str, slot: Slot) -> Self {
        Twim {
            name,
            slot,
            events: 0,
            pulses: 0,
            inten: 0,
            shorts: 0,
            errorsrc: 0,
            enable: 0,
            psel_scl: 0xFFFF_FFFF,
            psel_sda: 0xFFFF_FFFF,
            frequency: 0x0400_0000,
            rxd_ptr: 0,
            rxd_maxcnt: 0,
            rxd_amount: 0,
            rxd_list: 0,
            txd_ptr: 0,
            txd_maxcnt: 0,
            txd_amount: 0,
            txd_list: 0,
            address: 0,
            queue: VecDeque::new(),
            busy: false,
            rx_data: Vec::new(),
            rx_ptr: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Twim::new(self.name, self.slot);
    }

    pub fn irq(&self) -> bool {
        self.events & self.inten != 0
    }

    fn bit(&self) -> FracPeriod {
        // bit rate = FREQUENCY * 16 MHz / 2^32
        FracPeriod::new(TICKS_PER_SEC as u128 * (1u128 << 32), self.frequency.max(1) as u128 * 16_000_000)
    }

    fn at_bits(&self, start: Ticks, bits: u64) -> Ticks {
        self.bit().after(start, bits)
    }

    fn push(&mut self, t: Ticks, a: Action, env: &mut Env) {
        let pos = self.queue.iter().position(|(qt, _)| *qt > t).unwrap_or(self.queue.len());
        self.queue.insert(pos, (t, a));
        env.schedule(self.slot, self.queue[0].0);
    }

    fn pins(&self) -> Option<(u8, u8)> {
        Some((psel_pin(self.psel_scl)?, psel_pin(self.psel_sda)?))
    }

    pub fn read(&mut self, off: u32) -> u32 {
        if let Some(b) = event_bit(off) {
            return (self.events & b != 0) as u32;
        }
        match off {
            0x200 => self.shorts,
            0x300 | 0x304 | 0x308 => self.inten,
            0x4C4 => self.errorsrc,
            0x500 => self.enable,
            0x508 => self.psel_scl,
            0x50C => self.psel_sda,
            0x524 => self.frequency,
            0x534 => self.rxd_ptr,
            0x538 => self.rxd_maxcnt,
            0x53C => self.rxd_amount,
            0x540 => self.rxd_list,
            0x544 => self.txd_ptr,
            0x548 => self.txd_maxcnt,
            0x54C => self.txd_amount,
            0x550 => self.txd_list,
            0x588 => self.address,
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32, env: &mut Env) {
        if let Some(b) = event_bit(off) {
            if v & 1 == 0 {
                self.events &= !b;
            } else {
                self.events |= b;
            }
            return;
        }
        if inten_write(&mut self.inten, off, v) {
            self.inten &= 0x019C_0202;
            return;
        }
        let now = env.now;
        match off {
            0x000 if v & 1 != 0 => self.start_rx(now, env),
            0x008 if v & 1 != 0 => self.start_tx(now, env),
            0x014 if v & 1 != 0 => {
                // STOP: after the byte in progress, or now when the bus is held
                let t = self.queue.iter().filter(|(_, a)| matches!(a, Action::TxDone | Action::RxDone)).map(|(t, _)| *t).next();
                self.queue.retain(|(_, a)| matches!(a, Action::Event(_)));
                let start = t.unwrap_or(now).max(now);
                let at = self.at_bits(start, 1);
                self.push(at, Action::Stopped, env);
            }
            0x01C if v & 1 != 0 => self.ev(SUSPENDED),
            0x020 if v & 1 != 0 => {}
            0x200 => self.shorts = v & 0x1780,
            0x4C4 => self.errorsrc &= !v,
            0x500 => {
                self.enable = v & 0xF;
                if self.enable != 0 && self.enable != 6 {
                    env.warn.warn(now, format!("{}: ENABLE={} (SPIM/SPIS/TWIS/legacy) is not modelled", self.name, self.enable));
                }
            }
            0x508 => self.psel_scl = v,
            0x50C => self.psel_sda = v,
            0x524 => self.frequency = v,
            0x534 => self.rxd_ptr = v,
            0x538 => self.rxd_maxcnt = v & 0xFFFF,
            0x540 => self.rxd_list = v & 7,
            0x544 => self.txd_ptr = v,
            0x548 => self.txd_maxcnt = v & 0xFFFF,
            0x550 => self.txd_list = v & 7,
            0x588 => self.address = v & 0x7F,
            _ => {}
        }
    }

    fn start_tx(&mut self, t: Ticks, env: &mut Env) {
        if self.enable != 6 {
            return;
        }
        self.busy = true;
        self.ev(TXSTARTED);
        let n = self.txd_maxcnt as usize;
        let mut data = vec![0u8; n];
        env.dma_read(self.txd_ptr, &mut data, self.name);
        let res = match self.pins() {
            Some((scl, sda)) => env.board.i2c_write(t, scl, sda, self.address as u8, &data),
            None => I2cResult::AddrNack,
        };
        // start condition + address byte = 10 bit times, then 9 per data byte
        match res {
            I2cResult::AddrNack => {
                self.txd_amount = 0;
                let at = self.at_bits(t, 10);
                self.errorsrc |= 2;
                self.push(at, Action::Event(ERROR), env);
            }
            I2cResult::DataNack(i) => {
                self.txd_amount = i as u32;
                let at = self.at_bits(t, 10 + 9 * (i as u64 + 1));
                self.errorsrc |= 4;
                self.push(at, Action::Event(ERROR), env);
            }
            I2cResult::Ack => {
                let last = self.at_bits(t, 10 + 9 * (n.max(1) as u64 - 1));
                self.push(last, Action::Event(LASTTX), env);
                let done = self.at_bits(t, 10 + 9 * n as u64);
                self.push(done, Action::TxDone, env);
            }
        }
    }

    fn start_rx(&mut self, t: Ticks, env: &mut Env) {
        if self.enable != 6 {
            return;
        }
        self.busy = true;
        self.ev(RXSTARTED);
        let m = self.rxd_maxcnt as usize;
        let mut data = vec![0u8; m];
        let res = match self.pins() {
            Some((scl, sda)) => env.board.i2c_read(t, scl, sda, self.address as u8, &mut data),
            None => I2cResult::AddrNack,
        };
        match res {
            I2cResult::Ack => {
                self.rx_data = data;
                self.rx_ptr = self.rxd_ptr;
                let last = self.at_bits(t, 10 + 9 * (m.max(1) as u64 - 1));
                self.push(last, Action::Event(LASTRX), env);
                let done = self.at_bits(t, 10 + 9 * m as u64);
                self.push(done, Action::RxDone, env);
            }
            _ => {
                self.rxd_amount = 0;
                let at = self.at_bits(t, 10);
                self.errorsrc |= 2;
                self.push(at, Action::Event(ERROR), env);
            }
        }
    }

    pub fn event(&mut self, env: &mut Env) {
        let now = env.now;
        while let Some(&(t, a)) = self.queue.front() {
            if t > now {
                break;
            }
            self.queue.pop_front();
            match a {
                Action::Event(e) => self.ev(e),
                Action::TxDone => {
                    self.txd_amount = self.txd_maxcnt;
                    if self.txd_list == 1 {
                        self.txd_ptr = self.txd_ptr.wrapping_add(self.txd_maxcnt);
                    }
                    if self.shorts & SHORT_LASTTX_STARTRX != 0 {
                        self.start_rx(t, env);
                    } else if self.shorts & SHORT_LASTTX_STOP != 0 {
                        let at = self.at_bits(t, 1);
                        self.push(at, Action::Stopped, env);
                    } else if self.shorts & SHORT_LASTTX_SUSPEND != 0 {
                        self.ev(SUSPENDED);
                    }
                }
                Action::RxDone => {
                    let data = std::mem::take(&mut self.rx_data);
                    env.dma_write(self.rx_ptr, &data, self.name);
                    self.rxd_amount = data.len() as u32;
                    if self.rxd_list == 1 {
                        self.rxd_ptr = self.rxd_ptr.wrapping_add(self.rxd_maxcnt);
                    }
                    if self.shorts & SHORT_LASTRX_STOP != 0 {
                        let at = self.at_bits(t, 1);
                        self.push(at, Action::Stopped, env);
                    } else if self.shorts & SHORT_LASTRX_STARTTX != 0 {
                        self.start_tx(t, env);
                    }
                }
                Action::Stopped => {
                    self.busy = false;
                    self.ev(STOPPED);
                }
            }
        }
        if let Some(&(t, _)) = self.queue.front() {
            env.schedule(self.slot, t);
        }
    }
}
