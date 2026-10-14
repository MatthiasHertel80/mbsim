//! Register-level model of the Nordic nRF52833: memories, the memory map and the peripherals
//! the micro:bit V2 board support uses (CLOCK, POWER, NVMC, FICR, UICR, GPIO, TIMER, RTC,
//! UARTE, TWIM, SAADC, PWM, TEMP). Accesses to other peripherals are stored and reported once.
//!
//! [`System`] implements the core's [`mbsim_cpu::Bus`].

pub mod board;
pub mod clock;
pub mod egu;
pub mod env;
pub mod gpio;
pub mod gpiote;
pub mod mem;
pub mod nvmc;
pub mod ppi;
pub mod pwm;
pub mod rng;
pub mod rtc;
pub mod saadc;
pub mod temp;
pub mod timer;
pub mod twim;
pub mod uarte;
pub mod wdt;

use board::BoardIo;
use clock::{ClockPower, RESETREAS_DOG};
use egu::Egu;
use env::{slot, Env, Warnings};
use gpio::Gpio;
use gpiote::Gpiote;
use mbsim_core::{Scheduler, Ticks};
use mbsim_cpu::{Bus, BusError};
use mem::{Memory, CODE_RAM_BASE, FICR_BASE, FLASH_SIZE, UICR_BASE};
use nvmc::Nvmc;
use ppi::Ppi;
use pwm::Pwm;
use rng::Rng;
use rtc::Rtc;
use saadc::Saadc;
use std::collections::{HashMap, VecDeque};
use temp::Temp;
use timer::Timer;
use twim::Twim;
use uarte::Uarte;
use wdt::Wdt;

/// Names of the peripheral IDs (address bits 17:12) for messages.
pub fn peripheral_name(id: u32) -> &'static str {
    match id {
        0x00 => "CLOCK/POWER",
        0x01 => "RADIO",
        0x02 => "UARTE0",
        0x03 => "TWIM0/SPIM0",
        0x04 => "TWIM1/SPIM1",
        0x05 => "NFCT",
        0x06 => "GPIOTE",
        0x07 => "SAADC",
        0x08 => "TIMER0",
        0x09 => "TIMER1",
        0x0A => "TIMER2",
        0x0B => "RTC0",
        0x0C => "TEMP",
        0x0D => "RNG",
        0x0E => "ECB",
        0x0F => "AAR/CCM",
        0x10 => "WDT",
        0x11 => "RTC1",
        0x12 => "QDEC",
        0x13 => "COMP/LPCOMP",
        0x14..=0x19 => "EGU/SWI",
        0x1A => "TIMER3",
        0x1B => "TIMER4",
        0x1C => "PWM0",
        0x1D => "PDM",
        0x1E => "NVMC/ACL",
        0x1F => "PPI",
        0x20 => "MWU",
        0x21 => "PWM1",
        0x22 => "PWM2",
        0x23 => "SPIM2",
        0x24 => "RTC2",
        0x25 => "I2S",
        0x26 => "FPU",
        0x27 => "USBD",
        0x28 => "UARTE1",
        0x2D => "PWM3",
        0x2F => "SPIM3",
        _ => "reserved",
    }
}

/// Offsets of documented registers of the modelled peripherals end below 0xA00 (POWER RAM[]);
/// higher offsets are undocumented (errata workarounds of SystemInit) and are stored only.
const UNDOCUMENTED_OFFSET: u32 = 0xA00;

/// The device: memories and peripherals, with a board attached.
pub struct System<B: BoardIo> {
    pub mem: Memory,
    pub sched: Scheduler,
    pub board: B,
    pub warn: Warnings,
    pub clock: ClockPower,
    pub nvmc: Nvmc,
    pub gpio: Gpio,
    pub timer: [Timer; 5],
    pub uarte0: Uarte,
    pub twim: [Twim; 2],
    pub saadc: Saadc,
    pub pwm: [Pwm; 4],
    pub temp: Temp,
    pub rtc: [Rtc; 3],
    pub gpiote: Gpiote,
    pub ppi: Ppi,
    pub egu: [Egu; 6],
    pub rng: Rng,
    pub wdt: Wdt,
    /// A device reset requested by a peripheral (WDT), with the RESETREAS bits.
    pub reset_request: Option<u32>,
    routing: bool,
    /// Register storage of peripherals that are not modelled and of undocumented registers.
    pub unimpl: HashMap<u32, u32>,
    reported: [bool; 64],
    irq_lines: u64,
    irq_dirty: bool,
    deadline: Ticks,
    /// Next time the board changes an input by itself.
    board_next: Ticks,
    stall: Option<Ticks>,
    /// Flash wait cycles per instruction fetch (calibration parameter).
    pub flash_wait: u32,
    /// Bytes from the host to the target UART (the interface MCU transmits them in sequence).
    host_uart: VecDeque<u8>,
    host_uart_busy_until: Ticks,
    /// Target pin on which the interface MCU transmits (micro:bit: P1.08).
    pub host_uart_pin: Option<u8>,
}

impl<B: BoardIo> System<B> {
    pub fn new(board: B) -> Self {
        let mut s = System {
            mem: Memory::new(),
            sched: Scheduler::new(slot::COUNT),
            board,
            warn: Warnings::default(),
            clock: ClockPower::new(),
            nvmc: Nvmc::new(),
            gpio: Gpio::new(),
            timer: std::array::from_fn(|i| Timer::new(i, slot::TIMER[i])),
            uarte0: Uarte::new("UARTE0", slot::UARTE0_TX, slot::UARTE0_RX, slot::UARTE0_STOP),
            twim: [Twim::new("TWIM0", slot::TWIM[0]), Twim::new("TWIM1", slot::TWIM[1])],
            saadc: Saadc::new(slot::SAADC),
            pwm: std::array::from_fn(|i| Pwm::new(i, slot::PWM[i])),
            temp: Temp::new(slot::TEMP),
            rtc: std::array::from_fn(|i| Rtc::new(i, slot::RTC[i])),
            gpiote: Gpiote::default(),
            ppi: Ppi::new(),
            egu: Default::default(),
            rng: Rng::new(slot::RNG),
            wdt: Wdt::new(slot::WDT),
            reset_request: None,
            routing: false,
            unimpl: HashMap::new(),
            reported: [false; 64],
            irq_lines: 0,
            irq_dirty: true,
            deadline: Ticks::MAX,
            board_next: Ticks::MAX,
            stall: None,
            flash_wait: 0,
            host_uart: VecDeque::new(),
            host_uart_busy_until: 0,
            host_uart_pin: None,
        };
        s.gpio.reset(0, &mut s.board);
        s
    }

    /// Resets all peripherals. A system reset keeps RAM, RESETREAS and GPREGRET; the memories
    /// (flash, UICR) are non-volatile anyway.
    pub fn reset(&mut self, now: Ticks, system_reset: bool, reason: u32) {
        self.sched.clear();
        self.clock.reset(system_reset);
        self.clock.resetreas |= reason;
        self.nvmc = Nvmc::new();
        for t in &mut self.timer {
            t.reset();
        }
        self.uarte0.reset();
        for t in &mut self.twim {
            t.reset();
        }
        self.saadc.reset();
        for p in &mut self.pwm {
            p.reset();
        }
        self.temp.reset();
        for r in &mut self.rtc {
            r.reset();
        }
        self.gpiote = Gpiote::default();
        self.ppi = Ppi::new();
        self.egu = Default::default();
        self.rng.reset();
        self.wdt.reset();
        self.reset_request = None;
        self.unimpl.clear();
        if !system_reset {
            self.mem.ram.fill(0);
        }
        self.gpio.reset(now, &mut self.board);
        self.irq_lines = 0;
        self.irq_dirty = true;
        self.host_uart_busy_until = now;
        if !self.host_uart.is_empty() {
            self.sched.schedule(slot::UART_HOST, now);
        }
        self.update_deadline();
    }

    fn update_deadline(&mut self) {
        self.deadline = self.sched.next_time().unwrap_or(Ticks::MAX).min(self.board_next);
    }

    /// Time of the next device or board event.
    pub fn next_event(&mut self) -> Ticks {
        self.sched.next_time().unwrap_or(Ticks::MAX).min(self.board_next)
    }

    fn refresh_board_next(&mut self, now: Ticks) {
        // only needed while something listens to board-driven pin changes
        self.board_next =
            if self.gpiote.config.iter().any(|c| c & 3 == 1) || self.gpio.sensing() { self.board.next_event(now) } else { Ticks::MAX };
    }

    fn irq_line(&self, irq: u32) -> bool {
        match irq {
            0 => self.clock.irq(),
            2 => self.uarte0.irq(),
            3 => self.twim[0].irq(),
            4 => self.twim[1].irq(),
            7 => self.saadc.irq(),
            8..=10 => self.timer[(irq - 8) as usize].irq(),
            11 => self.rtc[0].irq(),
            12 => self.temp.irq(),
            17 => self.rtc[1].irq(),
            26 => self.timer[3].irq(),
            27 => self.timer[4].irq(),
            28 => self.pwm[0].irq(),
            33 => self.pwm[1].irq(),
            34 => self.pwm[2].irq(),
            36 => self.rtc[2].irq(),
            45 => self.pwm[3].irq(),
            6 => self.gpiote.irq(),
            13 => self.rng.irq(),
            16 => self.wdt.irq(),
            20..=25 => self.egu[(irq - 20) as usize].irq(),
            _ => false,
        }
    }

    fn update_irq(&mut self, irq: u32) {
        if irq >= 48 {
            return;
        }
        let bit = 1u64 << irq;
        let new = if self.irq_line(irq) { self.irq_lines | bit } else { self.irq_lines & !bit };
        if new != self.irq_lines {
            self.irq_lines = new;
            self.irq_dirty = true;
        }
    }

    /// Processes all device events due at or before `now`.
    pub fn process_events(&mut self, now: Ticks) {
        if self.board_next <= now {
            let t = self.board_next;
            self.route_pulses(t);
            self.refresh_board_next(t);
        }
        while let Some((s, t)) = self.sched.pop_due(now) {
            let mut env = Env {
                now: t,
                sched: &mut self.sched,
                ram: &mut self.mem.ram,
                gpio: &mut self.gpio,
                board: &mut self.board,
                warn: &mut self.warn,
            };
            let irq = match s {
                slot::CLOCK_HF | slot::CLOCK_LF | slot::CLOCK_CAL | slot::CLOCK_CT => {
                    self.clock.event(s);
                    0
                }
                slot::UARTE0_TX => {
                    self.uarte0.tx_event(&mut env);
                    2
                }
                slot::UARTE0_RX => {
                    self.uarte0.rx_event(&mut env);
                    2
                }
                slot::UARTE0_STOP => {
                    self.uarte0.stop_event(&mut env);
                    2
                }
                slot::UART_HOST => {
                    self.host_uart_next(t);
                    2
                }
                slot::SAADC => {
                    self.saadc.event(&mut env);
                    7
                }
                slot::TEMP => {
                    self.temp.event(&mut env);
                    12
                }
                slot::RNG => {
                    self.rng.event(&mut env);
                    13
                }
                slot::WDT => {
                    if self.wdt.event(&mut env) {
                        self.reset_request = Some(RESETREAS_DOG);
                    }
                    16
                }
                _ => {
                    if let Some(i) = slot::TIMER.iter().position(|x| *x == s) {
                        self.timer[i].event(&mut env);
                        [8, 9, 10, 26, 27][i]
                    } else if let Some(i) = slot::TWIM.iter().position(|x| *x == s) {
                        self.twim[i].event(&mut env);
                        3 + i as u32
                    } else if let Some(i) = slot::PWM.iter().position(|x| *x == s) {
                        self.pwm[i].event(&mut env);
                        [28, 33, 34, 45][i]
                    } else if let Some(i) = slot::RTC.iter().position(|x| *x == s) {
                        self.rtc[i].event(&mut env);
                        [11, 17, 36][i]
                    } else {
                        64
                    }
                }
            };
            self.update_irq(irq);
            self.route_pulses(t);
        }
        self.refresh_board_next(now);
        self.update_deadline();
    }

    /// Takes the event pulses of all peripherals: (EVENTS register address) list.
    fn take_pulses(&mut self, out: &mut Vec<u32>) {
        let mut add = |base: u32, p: &mut u32| {
            let mut b = std::mem::take(p);
            while b != 0 {
                let n = b.trailing_zeros();
                b &= b - 1;
                out.push(base + 0x100 + 4 * n);
            }
        };
        add(0x4000_0000, &mut self.clock.pulses);
        add(0x4000_2000, &mut self.uarte0.pulses);
        add(0x4000_3000, &mut self.twim[0].pulses);
        add(0x4000_4000, &mut self.twim[1].pulses);
        add(0x4000_6000, &mut self.gpiote.pulses);
        add(0x4000_7000, &mut self.saadc.pulses);
        for (i, base) in [0x4000_8000u32, 0x4000_9000, 0x4000_A000, 0x4001_A000, 0x4001_B000].into_iter().enumerate() {
            add(base, &mut self.timer[i].pulses);
        }
        for (i, base) in [0x4000_B000u32, 0x4001_1000, 0x4002_4000].into_iter().enumerate() {
            add(base, &mut self.rtc[i].pulses);
        }
        add(0x4000_C000, &mut self.temp.pulses);
        add(0x4000_D000, &mut self.rng.pulses);
        add(0x4001_0000, &mut self.wdt.pulses);
        for i in 0..6 {
            add(0x4001_4000 + 0x1000 * i as u32, &mut self.egu[i].pulses);
        }
        for (i, base) in [0x4001_C000u32, 0x4002_1000, 0x4002_2000, 0x4002_D000].into_iter().enumerate() {
            add(base, &mut self.pwm[i].pulses);
        }
    }

    /// Routes event pulses through the PPI to tasks (and updates GPIOTE inputs).
    fn route_pulses(&mut self, now: Ticks) {
        if self.routing {
            return;
        }
        self.routing = true;
        if self.gpiote.config.iter().any(|c| c & 3 == 1) || self.gpio.sensing() {
            self.gpiote.update(now, &mut self.gpio, &mut self.board);
            self.update_irq(6);
        }
        let mut events = Vec::new();
        for _ in 0..16 {
            events.clear();
            self.take_pulses(&mut events);
            if events.is_empty() || !self.ppi.active() {
                break;
            }
            let mut tasks = Vec::new();
            for e in &events {
                self.ppi.route(*e, &mut tasks);
            }
            if tasks.is_empty() {
                break;
            }
            for t in tasks {
                self.peripheral_write(now, t, 1);
            }
        }
        self.routing = false;
    }

    /// The board inputs changed (buttons, touch): GPIOTE and SENSE see the new levels.
    pub fn inputs_changed(&mut self, now: Ticks) {
        self.route_pulses(now);
        self.update_deadline();
    }

    // ------------------------------------------------------------------ host UART

    /// Bytes from the host (USB serial of the interface MCU) to the target UART.
    pub fn host_uart_send(&mut self, now: Ticks, data: &[u8]) {
        self.host_uart.extend(data);
        if self.sched.when(slot::UART_HOST).is_none() {
            self.sched.schedule(slot::UART_HOST, now.max(self.host_uart_busy_until));
        }
        self.update_deadline();
    }

    fn host_uart_next(&mut self, t: Ticks) {
        let Some(pin) = self.host_uart_pin else {
            self.host_uart.clear();
            return;
        };
        let Some(b) = self.host_uart.pop_front() else { return };
        let end = self.uarte0.frame_period().after(t, 1);
        let mut env = Env {
            now: t,
            sched: &mut self.sched,
            ram: &mut self.mem.ram,
            gpio: &mut self.gpio,
            board: &mut self.board,
            warn: &mut self.warn,
        };
        self.uarte0.receive(pin, end, b, &mut env);
        self.host_uart_busy_until = end;
        if !self.host_uart.is_empty() {
            self.sched.schedule(slot::UART_HOST, end);
        }
    }

    // ------------------------------------------------------------------ MMIO

    fn report_unimplemented(&mut self, now: Ticks, id: u32, addr: u32) {
        if !self.reported[id as usize & 63] {
            self.reported[id as usize & 63] = true;
            self.warn.warn(now, format!("access to {} at {addr:#010x}: peripheral not modelled", peripheral_name(id)));
        }
    }

    fn peripheral_read(&mut self, now: Ticks, addr: u32, debug: bool) -> u32 {
        let v = self.peripheral_read_inner(now, addr, debug);
        if !debug {
            log::trace!(target: "mmio", "{:.6} R {addr:#010x} = {v:#010x}", mbsim_core::time::to_secs(now));
        }
        v
    }

    fn peripheral_read_inner(&mut self, now: Ticks, addr: u32, debug: bool) -> u32 {
        let id = (addr >> 12) & 0x3F;
        let off = addr & 0xFFC;
        if off >= UNDOCUMENTED_OFFSET || addr >= 0x4003_0000 {
            return self.unimpl.get(&(addr & !3)).copied().unwrap_or(0);
        }
        let v = match id {
            0x00 => self.clock.read(off),
            0x02 => self.uarte0.read(off),
            0x03 | 0x04 => self.twim[(id - 3) as usize].read(off),
            0x07 => self.saadc.read(off),
            0x08..=0x0A => self.timer[(id - 8) as usize].read(off),
            0x0B => self.rtc[0].read(off, now),
            0x0C => self.temp.read(off),
            0x11 => self.rtc[1].read(off, now),
            0x1A | 0x1B => self.timer[(id - 0x1A + 3) as usize].read(off),
            0x1C => self.pwm[0].read(off),
            0x1E => self.nvmc.read(off),
            0x21 | 0x22 => self.pwm[(id - 0x20) as usize].read(off),
            0x24 => self.rtc[2].read(off, now),
            0x2D => self.pwm[3].read(off),
            0x06 => self.gpiote.read(off),
            0x0D => self.rng.read(off),
            0x10 => self.wdt.read(off),
            0x14..=0x19 => self.egu[(id - 0x14) as usize].read(off),
            0x1F => self.ppi.read(off),
            0x20 => self.unimpl.get(&(addr & !3)).copied().unwrap_or(0),
            _ => {
                if !debug {
                    self.report_unimplemented(now, id, addr);
                }
                return self.unimpl.get(&(addr & !3)).copied().unwrap_or(0);
            }
        };
        v
    }

    fn peripheral_write(&mut self, now: Ticks, addr: u32, v: u32) {
        log::trace!(target: "mmio", "{:.6} W {addr:#010x} = {v:#010x}", mbsim_core::time::to_secs(now));
        let id = (addr >> 12) & 0x3F;
        let off = addr & 0xFFC;
        if off >= UNDOCUMENTED_OFFSET || addr >= 0x4003_0000 {
            self.unimpl.insert(addr & !3, v);
            return;
        }
        let mut env =
            Env { now, sched: &mut self.sched, ram: &mut self.mem.ram, gpio: &mut self.gpio, board: &mut self.board, warn: &mut self.warn };
        let irq = match id {
            0x00 => {
                self.clock.write(off, v, &mut env);
                0
            }
            0x02 => {
                self.uarte0.write(off, v, &mut env);
                2
            }
            0x03 | 0x04 => {
                self.twim[(id - 3) as usize].write(off, v, &mut env);
                id
            }
            0x07 => {
                self.saadc.write(off, v, &mut env);
                7
            }
            0x08..=0x0A => {
                self.timer[(id - 8) as usize].write(off, v, &mut env);
                id
            }
            0x0B => {
                self.rtc[0].write(off, v, &mut env);
                11
            }
            0x0C => {
                self.temp.write(off, v, &mut env);
                12
            }
            0x11 => {
                self.rtc[1].write(off, v, &mut env);
                17
            }
            0x1A | 0x1B => {
                self.timer[(id - 0x1A + 3) as usize].write(off, v, &mut env);
                id
            }
            0x1C => {
                self.pwm[0].write(off, v, &mut env);
                28
            }
            0x1E => {
                if let Some(t) = self.nvmc.write(off, v, &mut self.mem) {
                    self.stall = Some(now + t);
                }
                64
            }
            0x21 | 0x22 => {
                self.pwm[(id - 0x20) as usize].write(off, v, &mut env);
                id
            }
            0x24 => {
                self.rtc[2].write(off, v, &mut env);
                36
            }
            0x2D => {
                self.pwm[3].write(off, v, &mut env);
                45
            }
            0x06 => {
                self.gpiote.write(off, v, now, env.gpio, env.board);
                6
            }
            0x0D => {
                self.rng.write(off, v, &mut env);
                13
            }
            0x10 => {
                self.wdt.write(off, v, &mut env);
                16
            }
            0x14..=0x19 => {
                self.egu[(id - 0x14) as usize].write(off, v);
                20 + id - 0x14
            }
            0x1F => {
                self.ppi.write(off, v);
                64
            }
            0x20 => {
                // MWU: register storage only (no memory protection events)
                self.unimpl.insert(addr & !3, v);
                64
            }
            _ => {
                self.report_unimplemented(now, id, addr);
                self.unimpl.insert(addr & !3, v);
                64
            }
        };
        self.update_irq(irq);
        self.route_pulses(now);
        self.refresh_board_next(now);
        self.update_deadline();
    }

    fn gpio_port(addr: u32) -> Option<(usize, u32)> {
        let off = addr - gpio::P0_BASE;
        match off {
            0x500..=0x77F => Some((0, off)),
            0x800..=0xA7F => Some((1, off - 0x300)),
            _ => None,
        }
    }

    /// Loads an image byte (loader and debugger flash programming).
    pub fn load_byte(&mut self, addr: u32, b: u8) -> bool {
        self.mem.load_byte(addr, b)
    }
}

fn sub_word(word: u32, addr: u32, size: u8) -> u32 {
    let sh = (addr & 3) * 8;
    match size {
        1 => (word >> sh) & 0xFF,
        2 => (word >> sh) & 0xFFFF,
        _ => word,
    }
}

impl<B: BoardIo> Bus for System<B> {
    #[inline]
    fn read(&mut self, now: Ticks, addr: u32, size: u8, debug: bool) -> Result<u32, BusError> {
        if addr as u64 + size as u64 <= FLASH_SIZE as u64 {
            let a = addr as usize;
            return Ok(match size {
                1 => self.mem.flash[a] as u32,
                2 => u16::from_le_bytes([self.mem.flash[a], self.mem.flash[a + 1]]) as u32,
                _ => u32::from_le_bytes(self.mem.flash[a..a + 4].try_into().unwrap()),
            });
        }
        if let Some(o) = Memory::ram_offset(addr).filter(|o| o + size as usize <= self.mem.ram.len()) {
            let r = &self.mem.ram;
            return Ok(match size {
                1 => r[o] as u32,
                2 => u16::from_le_bytes([r[o], r[o + 1]]) as u32,
                _ => u32::from_le_bytes(r[o..o + 4].try_into().unwrap()),
            });
        }
        match addr {
            0x1000_0000..=0x1000_0FFF => {
                let w = self.mem.ficr[((addr - FICR_BASE) / 4) as usize];
                Ok(sub_word(w, addr, size))
            }
            0x1000_1000..=0x1000_1FFF => {
                let w = self.mem.uicr[((addr - UICR_BASE) / 4) as usize];
                Ok(sub_word(w, addr, size))
            }
            0x4000_0000..=0x4007_FFFF => {
                let w = self.peripheral_read(now, addr & !3, debug);
                Ok(sub_word(w, addr, size))
            }
            0x5000_0000..=0x5000_0FFF => match Self::gpio_port(addr & !3) {
                Some((port, off)) => {
                    let w = self.gpio.read(port, off, now, &mut self.board);
                    Ok(sub_word(w, addr, size))
                }
                None => Ok(0),
            },
            _ => Err(BusError),
        }
    }

    #[inline]
    fn write(&mut self, now: Ticks, addr: u32, size: u8, v: u32, debug: bool) -> Result<(), BusError> {
        if let Some(o) = Memory::ram_offset(addr).filter(|o| o + size as usize <= self.mem.ram.len()) {
            let r = &mut self.mem.ram;
            match size {
                1 => r[o] = v as u8,
                2 => r[o..o + 2].copy_from_slice(&(v as u16).to_le_bytes()),
                _ => r[o..o + 4].copy_from_slice(&v.to_le_bytes()),
            }
            return Ok(());
        }
        if addr < FLASH_SIZE || (UICR_BASE..UICR_BASE + 0x1000).contains(&addr) {
            if debug {
                for i in 0..size as u32 {
                    self.mem.load_byte(addr + i, (v >> (8 * i)) as u8);
                }
                return Ok(());
            }
            return match self.nvmc.flash_write(addr, size, v, &mut self.mem) {
                Ok(Some(t)) => {
                    self.stall = Some(now + t);
                    Ok(())
                }
                Ok(None) => Ok(()),
                Err(()) => Err(BusError),
            };
        }
        match addr {
            0x1000_0000..=0x1000_0FFF => Ok(()),
            0x4000_0000..=0x4007_FFFF => {
                let word = addr & !3;
                let v = match size {
                    4 => v,
                    _ => {
                        let sh = (addr & 3) * 8;
                        let mask = if size == 1 { 0xFF } else { 0xFFFF } << sh;
                        let old = self.peripheral_read(now, word, true);
                        (old & !mask) | ((v << sh) & mask)
                    }
                };
                self.peripheral_write(now, word, v);
                Ok(())
            }
            0x5000_0000..=0x5000_0FFF => {
                if let Some((port, off)) = Self::gpio_port(addr & !3) {
                    let v = if size == 4 { v } else { v << ((addr & 3) * 8) };
                    self.gpio.write(port, off, v, now, &mut self.board);
                    self.route_pulses(now);
                    self.refresh_board_next(now);
                    self.update_deadline();
                }
                Ok(())
            }
            _ => Err(BusError),
        }
    }

    #[inline]
    fn fetch16(&mut self, addr: u32) -> Result<u16, BusError> {
        if addr < FLASH_SIZE - 1 {
            let a = addr as usize;
            return Ok(u16::from_le_bytes([self.mem.flash[a], self.mem.flash[a + 1]]));
        }
        if let Some(o) = Memory::ram_offset(addr) {
            if o + 1 < self.mem.ram.len() {
                return Ok(u16::from_le_bytes([self.mem.ram[o], self.mem.ram[o + 1]]));
            }
        }
        let _ = CODE_RAM_BASE;
        Err(BusError)
    }

    #[inline]
    fn deadline(&self) -> Ticks {
        self.deadline
    }

    #[inline]
    fn take_irq_lines(&mut self) -> Option<u64> {
        if self.irq_dirty {
            self.irq_dirty = false;
            Some(self.irq_lines)
        } else {
            None
        }
    }

    #[inline]
    fn fetch_wait_cycles(&self, addr: u32) -> u32 {
        if addr < FLASH_SIZE {
            self.flash_wait
        } else {
            0
        }
    }

    fn take_stall(&mut self) -> Option<Ticks> {
        self.stall.take()
    }

    #[inline]
    fn code_cache_limit(&self) -> u32 {
        FLASH_SIZE
    }

    #[inline]
    fn code_generation(&self) -> u64 {
        self.mem.code_gen
    }
}
