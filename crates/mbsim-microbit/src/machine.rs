//! The simulated board: the core, the device and the board, advanced together in time.

use crate::board::MicrobitBoard;
use crate::pins;
use mbsim_core::Ticks;
use mbsim_cpu::{Cpu, Exit, HaltReason, ResetRequest};
use mbsim_loader::Image;
use mbsim_nrf52833::clock::{RESETREAS_LOCKUP, RESETREAS_RESETPIN, RESETREAS_SREQ};
use mbsim_nrf52833::System;

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The requested time was reached.
    Time,
    /// The core halted in debug state.
    Halted(HaltReason),
    /// The device is in System OFF (needs a reset).
    SystemOff,
    /// The program exited through semihosting.
    Exited,
}

pub struct Machine {
    pub cpu: Cpu,
    pub sys: System<MicrobitBoard>,
    /// Halt instead of resetting when the core locks up (debugging aid).
    pub halt_on_lockup: bool,
    /// Number of system resets since power-on.
    pub resets: u64,
    /// Serve semihosting calls (BKPT 0xAB) instead of halting.
    pub semihosting: bool,
    pub semihost_out: Vec<u8>,
    pub semihost_in: Vec<u8>,
    /// Exit code of a semihosting SYS_EXIT (or a scenario).
    pub exit_code: Option<i32>,
    /// Everything the target sent on its UART (last 1 MB), for scenario checks.
    pub uart_log: Vec<u8>,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        let mut sys = System::new(MicrobitBoard::new());
        sys.host_uart_pin = Some(pins::UART_RX);
        let mut m = Machine {
            cpu: Cpu::new(),
            sys,
            halt_on_lockup: false,
            resets: 0,
            semihosting: false,
            semihost_out: Vec::new(),
            semihost_in: Vec::new(),
            exit_code: None,
            uart_log: Vec::new(),
        };
        m.power_on();
        m
    }

    /// Writes an image into flash/UICR/RAM.
    pub fn load_image(&mut self, img: &Image) -> Result<(), String> {
        for seg in &img.segments {
            for (i, b) in seg.data.iter().enumerate() {
                let a = seg.addr + i as u32;
                if !self.sys.load_byte(a, *b) {
                    return Err(format!("image byte at {a:#010x} is outside flash, UICR and RAM"));
                }
            }
        }
        Ok(())
    }

    /// Erases flash and UICR (like a chip erase).
    pub fn erase_all(&mut self) {
        self.sys.mem.erase_all();
    }

    /// Power-on reset of everything.
    pub fn power_on(&mut self) {
        let now = self.cpu.now;
        self.sys.reset(now, false, 0);
        self.sys.board.lsm.reset();
        self.cpu.power_on_reset(&mut self.sys);
    }

    /// System reset (SYSRESETREQ, pin reset, lockup); keeps RAM and the debug state.
    pub fn system_reset(&mut self, reason: u32) {
        let now = self.cpu.now;
        self.resets += 1;
        self.sys.reset(now, true, reason);
        self.cpu.reset(&mut self.sys);
    }

    /// Reset button (pin reset through the interface MCU).
    pub fn reset_button(&mut self) {
        self.system_reset(RESETREAS_RESETPIN);
    }

    pub fn now(&self) -> Ticks {
        self.cpu.now
    }

    /// Runs until `until` or a halt.
    pub fn run_until(&mut self, until: Ticks) -> Stop {
        loop {
            self.sys.process_events(self.cpu.now);
            if self.exit_code.is_some() {
                return Stop::Exited;
            }
            if self.semihosting {
                self.cpu.c_debugen = true;
            }
            if let Some(reason) = self.sys.reset_request.take() {
                self.system_reset(reason);
            }
            if self.sys.clock.system_off {
                // System OFF: nothing runs until a reset
                self.cpu.advance_to(until);
                return Stop::SystemOff;
            }
            if self.cpu.now >= until {
                return Stop::Time;
            }
            match self.cpu.run(&mut self.sys, until) {
                Exit::Deadline => {}
                Exit::Sleeping => {
                    let next = self.sys.next_event().min(self.cpu.next_internal_event()).min(until);
                    self.cpu.advance_to(next);
                }
                Exit::Halted(HaltReason::Bkpt(0xAB)) if self.semihosting => {
                    if !self.semihost() {
                        return Stop::Exited;
                    }
                }
                Exit::Halted(r) => return Stop::Halted(r),
                Exit::Reset(r) => match r {
                    ResetRequest::System => self.system_reset(RESETREAS_SREQ),
                    ResetRequest::Lockup => {
                        if self.halt_on_lockup && self.cpu.c_debugen {
                            self.cpu.halted = Some(HaltReason::Lockup);
                            return Stop::Halted(HaltReason::Lockup);
                        }
                        self.system_reset(RESETREAS_LOCKUP);
                    }
                },
            }
        }
    }

    /// Takes the bytes the target sent on its UART (and semihosting console output).
    pub fn take_uart(&mut self) -> Vec<u8> {
        let mut v = std::mem::take(&mut self.sys.board.uart_out);
        v.append(&mut self.semihost_out);
        self.uart_log.extend_from_slice(&v);
        if self.uart_log.len() > 1 << 20 {
            let n = self.uart_log.len() - (1 << 19);
            self.uart_log.drain(..n);
        }
        v
    }

    /// UART output so far, including bytes not yet taken.
    pub fn uart_text(&self) -> String {
        let mut v = self.uart_log.clone();
        v.extend_from_slice(&self.sys.board.uart_out);
        v.extend_from_slice(&self.semihost_out);
        String::from_utf8_lossy(&v).to_string()
    }

    /// LED pattern lit at any time during `window` (rows of 5 booleans).
    pub fn sample_leds(&mut self, window: Ticks) -> [[bool; 5]; 5] {
        let mut lit = [[false; 5]; 5];
        let end = self.now() + window;
        let step = (window / 64).max(1);
        while self.now() < end {
            let t = (self.now() + step).min(end);
            if self.run_until(t) != Stop::Time {
                break;
            }
            for (r, row) in lit.iter_mut().enumerate() {
                for (c, l) in row.iter_mut().enumerate() {
                    *l |= self.sys.board.led_on(r, c);
                }
            }
        }
        lit
    }

    /// Fundamental frequency of the speaker output over `window` (0 = silent). The gate signal
    /// is averaged in 50 us bins (which demodulates PWM audio), and the rising zero crossings of
    /// the AC part are counted.
    pub fn measure_speaker_hz(&mut self, window: Ticks) -> f64 {
        let was = self.sys.board.log_pins;
        self.sys.board.log_pins = true;
        let start_len = self.sys.board.pin_log.len();
        let start = self.now();
        let mut level = self.sys.board.drive(crate::pins::SPEAKER).driven() == Some(true);
        let _ = self.run_until(start + window);
        let edges: Vec<(Ticks, bool)> = self.sys.board.pin_log[start_len..]
            .iter()
            .filter(|e| e.pin == crate::pins::SPEAKER)
            .map(|e| (e.t, e.drive.driven() == Some(true)))
            .collect();
        if !was {
            self.sys.board.pin_log.truncate(start_len);
        }
        self.sys.board.log_pins = was;
        let bin = mbsim_core::time::us(50);
        let nbins = (window / bin) as usize;
        let mut duty = vec![0f64; nbins];
        let mut t = start;
        let mut ei = 0;
        for (b, d) in duty.iter_mut().enumerate() {
            let end = start + (b as u64 + 1) * bin;
            let mut high = 0u64;
            while ei < edges.len() && edges[ei].0 < end {
                if level {
                    high += edges[ei].0.saturating_sub(t);
                }
                t = edges[ei].0.max(t);
                level = edges[ei].1;
                ei += 1;
            }
            if level {
                high += end - t;
            }
            t = end;
            *d = high as f64 / bin as f64;
        }
        let mean = duty.iter().sum::<f64>() / nbins.max(1) as f64;
        let var = duty.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / nbins.max(1) as f64;
        if var < 1e-4 {
            return 0.0;
        }
        let rising: Vec<usize> = (1..nbins).filter(|&i| duty[i - 1] < mean && duty[i] >= mean).collect();
        if rising.len() < 3 {
            return 0.0;
        }
        let span = (rising[rising.len() - 1] - rising[0]) as f64 * mbsim_core::time::to_secs(bin);
        (rising.len() - 1) as f64 / span
    }

    /// Sends bytes from the host to the target UART.
    pub fn uart_send(&mut self, data: &[u8]) {
        let now = self.cpu.now;
        self.sys.host_uart_send(now, data);
    }
}
