//! UARTE (UART with EasyDMA). The frames on TXD go to the board; frames arriving on RXD come
//! from the board through [`Uarte::receive`].
//!
//! The actual baud rate is 16 MHz / floor(2^32 / BAUDRATE), which reproduces the rates listed in
//! the PS (115200 -> 115108 baud).

use crate::env::{inten_write, Env};
use crate::gpio::{psel_pin, PinOverride};
use mbsim_core::time::{us, FracPeriod, TICKS_PER_SEC};
use mbsim_core::{Slot, Ticks};
use std::collections::VecDeque;

// event bits (INTEN positions)
const CTS: u32 = 1 << 0;
const NCTS: u32 = 1 << 1;
const RXDRDY: u32 = 1 << 2;
const ENDRX: u32 = 1 << 4;
const TXDRDY: u32 = 1 << 7;
const ENDTX: u32 = 1 << 8;
const ERROR: u32 = 1 << 9;
const RXTO: u32 = 1 << 17;
const RXSTARTED: u32 = 1 << 19;
const TXSTARTED: u32 = 1 << 20;
const TXSTOPPED: u32 = 1 << 22;

fn event_bit(off: u32) -> Option<u32> {
    Some(match off {
        0x100 => CTS,
        0x104 => NCTS,
        0x108 => RXDRDY,
        0x110 => ENDRX,
        0x11C => TXDRDY,
        0x120 => ENDTX,
        0x124 => ERROR,
        0x144 => RXTO,
        0x14C => RXSTARTED,
        0x150 => TXSTARTED,
        0x158 => TXSTOPPED,
        _ => return None,
    })
}

#[derive(Debug)]
pub struct Uarte {
    pub name: &'static str,
    pub tx_slot: Slot,
    pub rx_slot: Slot,
    pub stop_slot: Slot,
    pub events: u32,
    /// Events generated since the last collection (bit n = EVENTS at 0x100 + 4n).
    pub pulses: u32,
    pub inten: u32,
    pub shorts: u32,
    pub errorsrc: u32,
    pub enable: u32,
    pub psel: [u32; 4], // RTS, TXD, CTS, RXD
    pub baudrate: u32,
    pub config: u32,
    pub rxd_ptr: u32,
    pub rxd_maxcnt: u32,
    pub rxd_amount: u32,
    pub txd_ptr: u32,
    pub txd_maxcnt: u32,
    pub txd_amount: u32,
    // transmitter
    tx_buf: Vec<u8>,
    tx_index: usize,
    tx_start: Ticks,
    tx_active: bool,
    tx_stop_requested: bool,
    tx_pin: Option<u8>,
    // receiver
    rx_running: bool,
    rx_buf_active: bool,
    rx_cur_ptr: u32,
    rx_cur_max: u32,
    rx_count: u32,
    rx_fifo: VecDeque<u8>,
    /// Bytes arriving from the board with their completion times.
    rx_line: VecDeque<(Ticks, u8)>,
}

impl Uarte {
    /// Generates events: sets the event bits and records them as pulses for PPI.
    #[inline]
    fn ev(&mut self, bits: u32) {
        self.events |= bits;
        self.pulses |= bits;
    }

    pub fn new(name: &'static str, tx_slot: Slot, rx_slot: Slot, stop_slot: Slot) -> Self {
        Uarte {
            name,
            tx_slot,
            rx_slot,
            stop_slot,
            events: 0,
            pulses: 0,
            inten: 0,
            shorts: 0,
            errorsrc: 0,
            enable: 0,
            psel: [0xFFFF_FFFF; 4],
            baudrate: 0x0400_0000,
            config: 0,
            rxd_ptr: 0,
            rxd_maxcnt: 0,
            rxd_amount: 0,
            txd_ptr: 0,
            txd_maxcnt: 0,
            txd_amount: 0,
            tx_buf: Vec::new(),
            tx_index: 0,
            tx_start: 0,
            tx_active: false,
            tx_stop_requested: false,
            tx_pin: None,
            rx_running: false,
            rx_buf_active: false,
            rx_cur_ptr: 0,
            rx_cur_max: 0,
            rx_count: 0,
            rx_fifo: VecDeque::new(),
            rx_line: VecDeque::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Uarte::new(self.name, self.tx_slot, self.rx_slot, self.stop_slot);
    }

    pub fn irq(&self) -> bool {
        self.events & self.inten != 0
    }

    fn enabled(&self) -> bool {
        self.enable == 8
    }

    /// Actual baud rate of the BAUDRATE setting.
    pub fn baud(&self) -> f64 {
        let div = (1u64 << 32) / self.baudrate.max(1) as u64;
        16e6 / div.max(1) as f64
    }

    fn bits_per_frame(&self) -> u32 {
        let parity = (self.config >> 1) & 7 == 7;
        let stop2 = self.config & (1 << 4) != 0;
        1 + 8 + parity as u32 + 1 + stop2 as u32
    }

    /// Duration of one frame in ticks.
    pub fn frame_period(&self) -> FracPeriod {
        let div = (1u128 << 32) / self.baudrate.max(1) as u128;
        // ticks per bit = TICKS_PER_SEC * div / 16e6
        FracPeriod::new(TICKS_PER_SEC as u128 * div.max(1) * self.bits_per_frame() as u128, 16_000_000)
    }

    pub fn rx_pin(&self) -> Option<u8> {
        if self.enabled() {
            psel_pin(self.psel[3])
        } else {
            None
        }
    }

    pub fn read(&mut self, off: u32) -> u32 {
        if let Some(b) = event_bit(off) {
            return (self.events & b != 0) as u32;
        }
        match off {
            0x200 => self.shorts,
            0x300 | 0x304 | 0x308 => self.inten,
            0x480 => self.errorsrc,
            0x500 => self.enable,
            0x508 => self.psel[0],
            0x50C => self.psel[1],
            0x510 => self.psel[2],
            0x514 => self.psel[3],
            0x524 => self.baudrate,
            0x534 => self.rxd_ptr,
            0x538 => self.rxd_maxcnt,
            0x53C => self.rxd_amount,
            0x544 => self.txd_ptr,
            0x548 => self.txd_maxcnt,
            0x54C => self.txd_amount,
            0x56C => self.config,
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
            self.inten &= 0x005A_0397;
            return;
        }
        let now = env.now;
        match off {
            0x000 if v & 1 != 0 => self.start_rx(env),
            0x004 if v & 1 != 0 && self.rx_running => {
                // RXTO after the receiver stopped (up to four more bytes could arrive)
                let t = self.frame_period().after(now, 1);
                env.schedule(self.stop_slot, t);
            }
            0x008 if v & 1 != 0 => self.start_tx(env),
            0x00C if v & 1 != 0 => {
                if self.tx_active {
                    self.tx_stop_requested = true;
                } else {
                    self.ev(TXSTOPPED);
                }
            }
            0x02C if v & 1 != 0 => {
                // FLUSHRX: move the FIFO to the buffer, then ENDRX
                self.rx_cur_ptr = self.rxd_ptr;
                self.rx_cur_max = self.rxd_maxcnt & 0xFFFF;
                self.rx_count = 0;
                self.rx_buf_active = true;
                while let Some(b) = self.rx_fifo.pop_front() {
                    self.store_rx(b, env);
                }
                self.end_rx(env);
            }
            0x200 => self.shorts = v & 0x60,
            0x480 => self.errorsrc &= !v,
            0x500 => {
                let was = self.enabled();
                self.enable = v & 0xF;
                if self.enable != 0 && self.enable != 8 {
                    env.warn.warn(now, format!("{}: legacy UART mode (ENABLE={}) is not modelled", self.name, self.enable));
                }
                if was && !self.enabled() {
                    self.release_tx_pin(env);
                    self.rx_running = false;
                    self.tx_active = false;
                    env.cancel(self.tx_slot);
                } else if !was && self.enabled() {
                    self.claim_tx_pin(env);
                }
            }
            0x508 => self.psel[0] = v,
            0x50C => {
                self.release_tx_pin(env);
                self.psel[1] = v;
                if self.enabled() {
                    self.claim_tx_pin(env);
                }
            }
            0x510 => self.psel[2] = v,
            0x514 => self.psel[3] = v,
            0x524 => self.baudrate = v,
            0x534 => self.rxd_ptr = v,
            0x538 => self.rxd_maxcnt = v & 0xFFFF,
            0x544 => self.txd_ptr = v,
            0x548 => self.txd_maxcnt = v & 0xFFFF,
            0x56C => self.config = v & 0x1F,
            _ => {}
        }
    }

    fn claim_tx_pin(&mut self, env: &mut Env) {
        self.tx_pin = psel_pin(self.psel[1]);
        if let Some(p) = self.tx_pin {
            env.gpio.set_override(p, Some(PinOverride::output(true)), env.now, env.board);
        }
    }

    fn release_tx_pin(&mut self, env: &mut Env) {
        if let Some(p) = self.tx_pin.take() {
            env.gpio.set_override(p, None, env.now, env.board);
        }
    }

    fn start_tx(&mut self, env: &mut Env) {
        if !self.enabled() {
            return;
        }
        let n = (self.txd_maxcnt & 0xFFFF) as usize;
        let mut buf = vec![0u8; n];
        env.dma_read(self.txd_ptr, &mut buf, self.name);
        self.tx_buf = buf;
        self.tx_index = 0;
        self.txd_amount = 0;
        self.tx_active = true;
        self.tx_stop_requested = false;
        self.tx_start = env.now + us(1) / 4;
        self.ev(TXSTARTED);
        if n == 0 {
            self.tx_active = false;
            self.ev(ENDTX);
            return;
        }
        let t = self.frame_period().after(self.tx_start, 1);
        env.schedule(self.tx_slot, t);
    }

    /// End of a TX frame.
    pub fn tx_event(&mut self, env: &mut Env) {
        if !self.tx_active {
            return;
        }
        let b = self.tx_buf[self.tx_index];
        if let Some(p) = self.tx_pin {
            env.board.uart_tx(env.now, p, b);
        }
        self.tx_index += 1;
        self.txd_amount = self.tx_index as u32;
        self.ev(TXDRDY);
        if self.tx_index >= self.tx_buf.len() || self.tx_stop_requested {
            self.tx_active = false;
            self.ev(ENDTX);
            if self.tx_stop_requested {
                self.ev(TXSTOPPED);
            }
            return;
        }
        let t = self.frame_period().after(self.tx_start, self.tx_index as u64 + 1);
        env.schedule(self.tx_slot, t);
    }

    fn start_rx(&mut self, env: &mut Env) {
        if !self.enabled() {
            return;
        }
        self.rx_running = true;
        self.rx_cur_ptr = self.rxd_ptr;
        self.rx_cur_max = self.rxd_maxcnt & 0xFFFF;
        self.rx_count = 0;
        self.rx_buf_active = true;
        self.ev(RXSTARTED);
        while self.rx_buf_active {
            match self.rx_fifo.pop_front() {
                Some(b) => self.store_rx(b, env),
                None => break,
            }
        }
    }

    fn store_rx(&mut self, b: u8, env: &mut Env) {
        if self.rx_count < self.rx_cur_max {
            env.dma_write(self.rx_cur_ptr + self.rx_count, &[b], self.name);
            self.rx_count += 1;
            self.rxd_amount = self.rx_count;
        }
        if self.rx_count >= self.rx_cur_max {
            self.end_rx(env);
        }
    }

    fn end_rx(&mut self, env: &mut Env) {
        self.rx_buf_active = false;
        self.rxd_amount = self.rx_count;
        self.ev(ENDRX);
        if self.shorts & (1 << 5) != 0 {
            self.start_rx(env);
        } else if self.shorts & (1 << 6) != 0 {
            self.rx_running = false;
            self.ev(RXTO);
        }
    }

    /// A byte arrives on `pin` with its frame ending at `at` (from the board).
    pub fn receive(&mut self, pin: u8, at: Ticks, byte: u8, env: &mut Env) {
        if self.rx_pin() != Some(pin) {
            return;
        }
        self.rx_line.push_back((at, byte));
        if let Some(&(t, _)) = self.rx_line.front() {
            env.schedule(self.rx_slot, t);
        }
    }

    /// End of an RX frame.
    pub fn rx_event(&mut self, env: &mut Env) {
        while let Some(&(t, b)) = self.rx_line.front() {
            if t > env.now {
                env.schedule(self.rx_slot, t);
                break;
            }
            self.rx_line.pop_front();
            if !self.rx_running {
                continue;
            }
            self.ev(RXDRDY);
            if self.rx_buf_active {
                self.store_rx(b, env);
            } else if self.rx_fifo.len() < 4 {
                self.rx_fifo.push_back(b);
            } else {
                self.errorsrc |= 1; // OVERRUN
                self.ev(ERROR);
            }
        }
    }

    /// RXTO after STOPRX.
    pub fn stop_event(&mut self, env: &mut Env) {
        if self.rx_buf_active {
            self.end_rx(env);
        }
        self.rx_running = false;
        self.rx_buf_active = false;
        self.ev(RXTO);
    }
}
