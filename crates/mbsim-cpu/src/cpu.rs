//! Core state, the exception model (DDI 0403E B1.5) and the run loop.

use crate::bus::{Bus, BusError, ResetRequest};
use crate::decode::{decode, is_32bit, Instr};
use crate::scs::{Dwt, Fpb, Itm, Mpu, SysTick};
use mbsim_core::time::{ceil_cpu_cycle, TICKS_PER_CPU_CYCLE};
use mbsim_core::Ticks;

/// Number of external interrupts of the nRF52833.
pub const NUM_IRQ: usize = 48;
/// Number of exceptions (16 system exceptions + IRQs).
pub const NUM_EXC: usize = 16 + NUM_IRQ;

pub const EXC_RESET: usize = 1;
pub const EXC_NMI: usize = 2;
pub const EXC_HARDFAULT: usize = 3;
pub const EXC_MEMMANAGE: usize = 4;
pub const EXC_BUSFAULT: usize = 5;
pub const EXC_USAGEFAULT: usize = 6;
pub const EXC_SVCALL: usize = 11;
pub const EXC_DEBUGMON: usize = 12;
pub const EXC_PENDSV: usize = 14;
pub const EXC_SYSTICK: usize = 15;

// CFSR bits
pub const CFSR_IACCVIOL: u32 = 1 << 0;
pub const CFSR_DACCVIOL: u32 = 1 << 1;
pub const CFSR_MUNSTKERR: u32 = 1 << 3;
pub const CFSR_MSTKERR: u32 = 1 << 4;
pub const CFSR_MLSPERR: u32 = 1 << 5;
pub const CFSR_MMARVALID: u32 = 1 << 7;
pub const CFSR_IBUSERR: u32 = 1 << 8;
pub const CFSR_PRECISERR: u32 = 1 << 9;
pub const CFSR_IMPRECISERR: u32 = 1 << 10;
pub const CFSR_UNSTKERR: u32 = 1 << 11;
pub const CFSR_STKERR: u32 = 1 << 12;
pub const CFSR_LSPERR: u32 = 1 << 13;
pub const CFSR_BFARVALID: u32 = 1 << 15;
pub const CFSR_UNDEFINSTR: u32 = 1 << 16;
pub const CFSR_INVSTATE: u32 = 1 << 17;
pub const CFSR_INVPC: u32 = 1 << 18;
pub const CFSR_NOCP: u32 = 1 << 19;
pub const CFSR_UNALIGNED: u32 = 1 << 24;
pub const CFSR_DIVBYZERO: u32 = 1 << 25;

pub const HFSR_VECTTBL: u32 = 1 << 1;
pub const HFSR_FORCED: u32 = 1 << 30;
pub const HFSR_DEBUGEVT: u32 = 1 << 31;

pub const DFSR_HALTED: u32 = 1 << 0;
pub const DFSR_BKPT: u32 = 1 << 1;
pub const DFSR_DWTTRAP: u32 = 1 << 2;
pub const DFSR_VCATCH: u32 = 1 << 3;
pub const DFSR_EXTERNAL: u32 = 1 << 4;

/// A synchronous fault or another reason to leave normal execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    /// UsageFault with the CFSR bit.
    Usage(u32),
    /// BusFault with CFSR bits and the fault address (precise errors).
    Bus(u32, Option<u32>),
    /// MemManage fault with CFSR bits and the fault address.
    Mem(u32, Option<u32>),
    Bkpt(u8),
    Svc,
}

/// Why the core halted in debug state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HaltReason {
    /// Halt request from the debugger (C_HALT).
    Request,
    /// Single step completed.
    Step,
    /// BKPT instruction.
    Bkpt(u8),
    /// FPB comparator or debugger breakpoint.
    Breakpoint,
    /// DWT watchpoint: address and access kind (0 = read, 1 = write).
    Watchpoint(u32, u8),
    /// Vector catch (reset or fault).
    VectorCatch,
    Lockup,
}

/// Result of [`Cpu::run`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The time limit or the bus deadline was reached.
    Deadline,
    /// The core sleeps (WFI/WFE) until an event.
    Sleeping,
    /// The core halted in debug state.
    Halted(HaltReason),
    /// The core requests a system reset.
    Reset(ResetRequest),
}

/// A data watchpoint set by the debugger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watch {
    pub addr: u32,
    pub len: u32,
    /// bit 0 = read, bit 1 = write
    pub kind: u8,
}

/// Core state of a Cortex-M4F.
pub struct Cpu {
    /// R0..R14; R13 holds the active stack pointer.
    pub r: [u32; 16],
    /// Address of the next instruction to execute.
    pub pc: u32,
    /// The banked stack pointer that is not active.
    pub sp_alt: u32,
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    pub q: bool,
    pub ge: u8,
    /// ITSTATE (EPSR.IT).
    pub it: u8,
    /// EPSR.T
    pub t: bool,
    /// IPSR exception number.
    pub ipsr: u32,
    pub primask: bool,
    pub faultmask: bool,
    pub basepri: u8,
    pub npriv: bool,
    pub spsel: bool,
    pub fpca: bool,
    /// Floating-point registers S0..S31.
    pub s: [u32; 32],
    pub fpscr: u32,

    /// Current time; advanced by the executed instructions.
    pub now: Ticks,
    /// Retired instructions.
    pub instret: u64,

    // exception state
    pub active: u64,
    pub pending: u64,
    /// NVIC enables of the external interrupts (bit n = IRQ n).
    pub irq_enabled: u64,
    /// Interrupt line levels from the device.
    pub irq_lines: u64,
    pub prio: [u8; NUM_EXC],
    exc_dirty: bool,
    exc_ready: Option<usize>,

    // SCB
    pub vtor: u32,
    pub prigroup: u32,
    pub scr: u32,
    pub ccr: u32,
    pub shcsr_ena: u32,
    pub cfsr: u32,
    pub hfsr: u32,
    pub dfsr: u32,
    pub mmfar: u32,
    pub bfar: u32,
    pub afsr: u32,
    pub actlr: u32,
    pub cpacr: u32,
    pub fpccr: u32,
    pub fpcar: u32,
    pub fpdscr: u32,

    pub systick: SysTick,
    pub mpu: Mpu,
    pub dwt: Dwt,
    pub fpb: Fpb,
    pub itm: Itm,

    // debug
    pub c_debugen: bool,
    pub c_halt: bool,
    pub c_step: bool,
    pub c_maskints: bool,
    pub demcr: u32,
    pub dcrdr: u32,
    pub s_retire_st: bool,
    pub s_reset_st: bool,
    pub halted: Option<HaltReason>,
    pub lockup: bool,
    /// Breakpoints of the debugger in addition to the FPB (unlimited).
    pub sw_breakpoints: Vec<u32>,
    /// Watchpoints of the debugger in addition to the DWT.
    pub watchpoints: Vec<Watch>,
    pub(crate) watch_hit: Option<(u32, u8)>,
    pub(crate) watch_active: bool,
    /// Skip breakpoint checks for the first instruction after a resume.
    pub(crate) skip_bp: bool,
    /// Extra debugger-requested step: halt after one instruction.
    pub step_request: bool,

    pub sleeping: bool,
    pub(crate) sleep_wfe: bool,
    pub event_reg: bool,
    pub(crate) excl: bool,
    pub(crate) next_pc: u32,
    pub(crate) exc_return: Option<u32>,
    pub(crate) reset_request: Option<ResetRequest>,
    /// Accumulated wait cycles from the bus for the current instruction.
    pub(crate) extra_cycles: u32,
    /// Number of exceptions taken (statistics).
    pub exceptions_taken: u64,
    /// Decoded instructions of the cacheable code memory (per halfword).
    icache: Vec<Instr>,
    icache_size: Vec<u8>,
    icache_gen: u64,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        let mut c = Cpu {
            r: [0; 16],
            pc: 0,
            sp_alt: 0,
            n: false,
            z: false,
            c: false,
            v: false,
            q: false,
            ge: 0,
            it: 0,
            t: true,
            ipsr: 0,
            primask: false,
            faultmask: false,
            basepri: 0,
            npriv: false,
            spsel: false,
            fpca: false,
            s: [0; 32],
            fpscr: 0,
            now: 0,
            instret: 0,
            active: 0,
            pending: 0,
            irq_enabled: 0,
            irq_lines: 0,
            prio: [0; NUM_EXC],
            exc_dirty: true,
            exc_ready: None,
            vtor: 0,
            prigroup: 0,
            scr: 0,
            ccr: 0x200,
            shcsr_ena: 0,
            cfsr: 0,
            hfsr: 0,
            dfsr: 0,
            mmfar: 0,
            bfar: 0,
            afsr: 0,
            actlr: 0,
            cpacr: 0,
            fpccr: 0xC000_0000,
            fpcar: 0,
            fpdscr: 0,
            systick: SysTick::default(),
            mpu: Mpu::default(),
            dwt: Dwt::default(),
            fpb: Fpb::default(),
            itm: Itm::default(),
            c_debugen: false,
            c_halt: false,
            c_step: false,
            c_maskints: false,
            demcr: 0,
            dcrdr: 0,
            s_retire_st: false,
            s_reset_st: false,
            halted: None,
            lockup: false,
            sw_breakpoints: Vec::new(),
            watchpoints: Vec::new(),
            watch_hit: None,
            watch_active: false,
            skip_bp: false,
            step_request: false,
            sleeping: false,
            sleep_wfe: false,
            event_reg: false,
            excl: false,
            next_pc: 0,
            exc_return: None,
            reset_request: None,
            extra_cycles: 0,
            exceptions_taken: 0,
            icache: Vec::new(),
            icache_size: Vec::new(),
            icache_gen: u64::MAX,
        };
        c.power_on_reset_debug();
        c
    }

    /// Resets the debug components (power-on reset only).
    fn power_on_reset_debug(&mut self) {
        self.c_debugen = false;
        self.c_halt = false;
        self.c_step = false;
        self.c_maskints = false;
        self.demcr = 0;
        self.dwt = Dwt::default();
        self.fpb = Fpb::default();
        self.itm = Itm::default();
    }

    /// Cold reset: core, NVIC, SCB and debug components.
    pub fn power_on_reset<B: Bus>(&mut self, bus: &mut B) {
        self.power_on_reset_debug();
        self.reset(bus);
    }

    /// Local (system) reset: core, NVIC and SCB. The debug components keep their state, and
    /// with DEMCR.VC_CORERESET the core halts before the first instruction.
    pub fn reset<B: Bus>(&mut self, bus: &mut B) {
        self.r = [0; 16];
        self.r[14] = 0xFFFF_FFFF;
        self.n = false;
        self.z = false;
        self.c = false;
        self.v = false;
        self.q = false;
        self.ge = 0;
        self.it = 0;
        self.ipsr = 0;
        self.primask = false;
        self.faultmask = false;
        self.basepri = 0;
        self.npriv = false;
        self.spsel = false;
        self.fpca = false;
        self.s = [0; 32];
        self.fpscr = 0;
        self.active = 0;
        self.pending = 0;
        self.irq_enabled = 0;
        self.prio = [0; NUM_EXC];
        self.vtor = 0;
        self.prigroup = 0;
        self.scr = 0;
        self.ccr = 0x200;
        self.shcsr_ena = 0;
        self.cfsr = 0;
        self.hfsr = 0;
        self.mmfar = 0;
        self.bfar = 0;
        self.afsr = 0;
        self.actlr = 0;
        self.cpacr = 0;
        self.fpccr = 0xC000_0000;
        self.fpcar = 0;
        self.fpdscr = 0;
        self.systick = SysTick::default();
        self.mpu = Mpu::default();
        self.lockup = false;
        self.sleeping = false;
        self.sleep_wfe = false;
        self.event_reg = false;
        self.excl = false;
        self.exc_return = None;
        self.reset_request = None;
        self.exc_dirty = true;
        self.s_reset_st = true;
        self.now = ceil_cpu_cycle(self.now);
        let sp = bus.read(self.now, 0, 4, true).unwrap_or(0);
        let rv = bus.read(self.now, 4, 4, true).unwrap_or(0);
        self.r[13] = sp & !3;
        self.sp_alt = 0;
        self.pc = rv & !1;
        self.t = rv & 1 == 1;
        if self.c_debugen && self.demcr & 1 != 0 {
            self.halted = Some(HaltReason::VectorCatch);
            self.dfsr |= DFSR_VCATCH;
        }
    }

    // ---------------------------------------------------------------- registers

    #[inline]
    pub fn handler_mode(&self) -> bool {
        self.ipsr != 0
    }

    #[inline]
    pub fn privileged(&self) -> bool {
        self.handler_mode() || !self.npriv
    }

    #[inline]
    fn using_psp(&self) -> bool {
        !self.handler_mode() && self.spsel
    }

    pub fn msp(&self) -> u32 {
        if self.using_psp() {
            self.sp_alt
        } else {
            self.r[13]
        }
    }

    pub fn psp(&self) -> u32 {
        if self.using_psp() {
            self.r[13]
        } else {
            self.sp_alt
        }
    }

    pub fn set_msp(&mut self, v: u32) {
        if self.using_psp() {
            self.sp_alt = v & !3;
        } else {
            self.r[13] = v & !3;
        }
    }

    pub fn set_psp(&mut self, v: u32) {
        if self.using_psp() {
            self.r[13] = v & !3;
        } else {
            self.sp_alt = v & !3;
        }
    }

    /// Changes the mode/stack selection and swaps the banked SP when it changes.
    pub(crate) fn set_stack_state(&mut self, ipsr: u32, spsel: bool) {
        let before = self.using_psp();
        self.ipsr = ipsr;
        self.spsel = spsel;
        if self.using_psp() != before {
            std::mem::swap(&mut self.r[13], &mut self.sp_alt);
        }
    }

    pub fn apsr(&self) -> u32 {
        ((self.n as u32) << 31)
            | ((self.z as u32) << 30)
            | ((self.c as u32) << 29)
            | ((self.v as u32) << 28)
            | ((self.q as u32) << 27)
            | ((self.ge as u32) << 16)
    }

    pub fn set_apsr_nzcvq(&mut self, v: u32) {
        self.n = v >> 31 & 1 != 0;
        self.z = v >> 30 & 1 != 0;
        self.c = v >> 29 & 1 != 0;
        self.v = v >> 28 & 1 != 0;
        self.q = v >> 27 & 1 != 0;
    }

    pub fn epsr(&self) -> u32 {
        ((self.t as u32) << 24) | (((self.it & 3) as u32) << 25) | (((self.it >> 2) as u32) << 10)
    }

    /// The combined xPSR.
    pub fn xpsr(&self) -> u32 {
        self.apsr() | self.epsr() | (self.ipsr & 0x1FF)
    }

    /// Writes the xPSR as a debugger does (all fields).
    pub fn set_xpsr(&mut self, v: u32) {
        self.set_apsr_nzcvq(v);
        self.ge = ((v >> 16) & 0xF) as u8;
        self.t = (v >> 24) & 1 != 0;
        self.it = ((((v >> 10) & 0x3F) << 2) | ((v >> 25) & 3)) as u8;
        let ipsr = v & 0x1FF;
        let spsel = self.spsel;
        self.set_stack_state(ipsr, spsel);
        self.exc_dirty = true;
    }

    pub fn control(&self) -> u32 {
        (self.npriv as u32) | ((self.spsel as u32) << 1) | ((self.fpca as u32) << 2)
    }

    pub fn set_control(&mut self, v: u32) {
        self.npriv = v & 1 != 0;
        self.fpca = v & 4 != 0;
        let ipsr = self.ipsr;
        self.set_stack_state(ipsr, v & 2 != 0);
    }

    pub fn set_primask(&mut self, v: bool) {
        self.primask = v;
        self.exc_dirty = true;
    }

    pub fn set_faultmask(&mut self, v: bool) {
        self.faultmask = v;
        self.exc_dirty = true;
    }

    pub fn set_basepri(&mut self, v: u8) {
        self.basepri = v & 0xE0;
        self.exc_dirty = true;
    }

    /// Reads a core register by its DCRSR number: 0-12, 13 SP, 14 LR, 15 PC (debug return
    /// address), 16 xPSR, 17 MSP, 18 PSP, 20 CONTROL/FAULTMASK/BASEPRI/PRIMASK, 33 FPSCR,
    /// 64..95 S0..S31.
    pub fn read_core_reg(&self, sel: u32) -> Option<u32> {
        Some(match sel {
            0..=12 | 14 => self.r[sel as usize],
            13 => self.r[13],
            15 => self.pc,
            16 => self.xpsr(),
            17 => self.msp(),
            18 => self.psp(),
            20 => (self.control() << 24) | ((self.faultmask as u32) << 16) | ((self.basepri as u32) << 8) | self.primask as u32,
            33 => self.fpscr,
            64..=95 => self.s[(sel - 64) as usize],
            _ => return None,
        })
    }

    pub fn write_core_reg(&mut self, sel: u32, v: u32) -> bool {
        match sel {
            0..=12 | 14 => self.r[sel as usize] = v,
            13 => self.r[13] = v & !3,
            15 => {
                self.pc = v & !1;
                self.skip_bp = false;
            }
            16 => self.set_xpsr(v),
            17 => self.set_msp(v),
            18 => self.set_psp(v),
            20 => {
                self.set_primask(v & 1 != 0);
                self.set_basepri((v >> 8) as u8);
                self.set_faultmask((v >> 16) & 1 != 0);
                self.set_control((v >> 24) & 7);
            }
            33 => self.fpscr = v & crate::fpu::FPSCR_MASK,
            64..=95 => self.s[(sel - 64) as usize] = v,
            _ => return false,
        }
        true
    }

    // ---------------------------------------------------------------- exceptions

    /// Group priority of a priority value under PRIGROUP.
    #[inline]
    fn group(&self, p: i32) -> i32 {
        if p < 0 {
            return p;
        }
        let mask = !((2u32 << self.prigroup) - 1) & 0xFF;
        (p as u32 & mask) as i32
    }

    #[inline]
    fn exc_priority(&self, n: usize) -> i32 {
        match n {
            EXC_RESET => -3,
            EXC_NMI => -2,
            EXC_HARDFAULT => -1,
            _ => self.prio[n] as i32,
        }
    }

    /// `ExecutionPriority()`, optionally ignoring PRIMASK (for WFI wake-up).
    fn execution_priority_masked(&self, with_primask: bool) -> i32 {
        let mut highest = 256;
        let mut a = self.active;
        while a != 0 {
            let n = a.trailing_zeros() as usize;
            a &= a - 1;
            highest = highest.min(self.group(self.exc_priority(n)));
        }
        let mut boosted = 256;
        if self.primask && with_primask {
            boosted = 0;
        }
        if self.basepri != 0 {
            boosted = boosted.min(self.group(self.basepri as i32));
        }
        if self.faultmask {
            boosted = -1;
        }
        highest.min(boosted)
    }

    pub fn execution_priority(&self) -> i32 {
        self.execution_priority_masked(true)
    }

    /// The pending exceptions that may be taken, before priority comparison.
    fn pending_candidates(&self) -> u64 {
        let mut p = self.pending & (0xFFFF | (self.irq_enabled << 16));
        if self.c_maskints && self.c_debugen {
            p &= !((1 << EXC_PENDSV) | (1 << EXC_SYSTICK) | (!0u64 << 16));
        }
        p
    }

    /// Highest-priority pending exception regardless of the execution priority.
    fn highest_pending(&self) -> Option<usize> {
        let mut p = self.pending_candidates();
        let mut best: Option<(i32, usize)> = None;
        while p != 0 {
            let n = p.trailing_zeros() as usize;
            p &= p - 1;
            let pr = self.exc_priority(n);
            if best.is_none_or(|(bp, _)| pr < bp) {
                best = Some((pr, n));
            }
        }
        best.map(|(_, n)| n)
    }

    fn compute_ready(&self, with_primask: bool) -> Option<usize> {
        let n = self.highest_pending()?;
        if self.group(self.exc_priority(n)) < self.execution_priority_masked(with_primask) {
            Some(n)
        } else {
            None
        }
    }

    pub(crate) fn mark_exc_dirty(&mut self) {
        self.exc_dirty = true;
    }

    /// Forces a re-evaluation of the pending exceptions (after debugger changes).
    pub fn mark_dirty(&mut self) {
        self.exc_dirty = true;
    }

    /// The next instruction executes without a breakpoint check.
    pub fn skip_bp_once(&mut self) {
        self.skip_bp = true;
    }

    /// Re-evaluates whether data accesses must be checked for watchpoints.
    pub fn refresh_watch(&mut self) {
        self.watch_active = !self.watchpoints.is_empty() || self.dwt.function.iter().any(|f| matches!(f & 0xF, 5..=7));
    }

    /// The exception that would be taken now.
    fn ready_exception(&mut self) -> Option<usize> {
        if self.exc_dirty {
            self.exc_dirty = false;
            self.exc_ready = self.compute_ready(true);
        }
        self.exc_ready
    }

    /// Highest pending exception number for ICSR.VECTPENDING (0 if none).
    pub fn vect_pending(&self) -> u32 {
        self.highest_pending().unwrap_or(0) as u32
    }

    pub fn set_pending(&mut self, n: usize) {
        self.pending |= 1 << n;
        self.exc_dirty = true;
    }

    pub fn clear_pending(&mut self, n: usize) {
        self.pending &= !(1 << n);
        // level-sensitive interrupts stay pending while their line is high and they are not active
        if n >= 16 && self.irq_lines & (1 << (n - 16)) != 0 && self.active & (1 << n) == 0 {
            self.pending |= 1 << n;
        }
        self.exc_dirty = true;
    }

    /// New interrupt line levels from the device.
    pub fn set_irq_lines(&mut self, lines: u64) {
        let rising = lines & !self.irq_lines;
        self.irq_lines = lines;
        // a high line makes the interrupt pending unless it is active
        let p = (lines & !(self.active >> 16)) | rising;
        if p != 0 {
            self.pending |= (p & ((1 << NUM_IRQ) - 1)) << 16;
            self.exc_dirty = true;
        }
    }

    /// Software pend of an IRQ (ISPR/STIR).
    pub fn pend_irq(&mut self, irq: usize) {
        if irq < NUM_IRQ {
            self.set_pending(16 + irq);
        }
    }

    fn return_address_frame(&self) -> u32 {
        self.pc
    }

    /// `PushStack` + `ExceptionTaken` for exception `n`; `ret` is the return address.
    pub(crate) fn take_exception<B: Bus>(&mut self, bus: &mut B, n: usize, ret: u32) {
        let fp_frame = self.fpca;
        let framesize: u32 = if fp_frame { 0x68 } else { 0x20 };
        let forcealign = fp_frame || self.ccr & 0x200 != 0;
        let sp = self.r[13];
        let align_bit = (sp >> 2) & 1 == 1 && forcealign;
        let mut frameptr = sp.wrapping_sub(framesize);
        if forcealign {
            frameptr &= !7;
        }
        let xpsr = (self.xpsr() & !(1 << 9)) | ((align_bit as u32) << 9);
        let vals = [self.r[0], self.r[1], self.r[2], self.r[3], self.r[12], self.r[14], ret, xpsr];
        let mut stk_err = false;
        for (i, v) in vals.iter().enumerate() {
            if self.write_mem_stack(bus, frameptr + 4 * i as u32, *v).is_err() {
                stk_err = true;
            }
        }
        if fp_frame {
            if self.fpccr & (1 << 30) == 0 {
                for i in 0..16 {
                    if self.write_mem_stack(bus, frameptr + 0x20 + 4 * i, self.s[i as usize]).is_err() {
                        stk_err = true;
                    }
                }
                let fpscr = self.fpscr;
                if self.write_mem_stack(bus, frameptr + 0x60, fpscr).is_err() {
                    stk_err = true;
                }
            } else {
                self.update_fpccr(frameptr);
            }
        }
        self.r[13] = frameptr;
        let lr = if self.handler_mode() {
            0xFFFF_FFF1
        } else if self.spsel {
            0xFFFF_FFFD
        } else {
            0xFFFF_FFF9
        };
        self.r[14] = if fp_frame { lr & !0x10 } else { lr };
        if stk_err {
            self.cfsr |= CFSR_STKERR;
        }
        self.exception_taken(bus, n);
        if stk_err {
            self.escalate_or_pend(EXC_BUSFAULT, self.shcsr_ena & (1 << 17) != 0);
        }
    }

    /// `UpdateFPCCR` for lazy FP stacking.
    fn update_fpccr(&mut self, frameptr: u32) {
        let mut v = self.fpccr & 0xC000_0000;
        v |= 1; // LSPACT
        if !self.privileged() {
            v |= 1 << 1; // USER
        }
        if !self.handler_mode() {
            v |= 1 << 3; // THREAD
        }
        let ep = self.execution_priority();
        if ep > -1 {
            v |= 1 << 4; // HFRDY
        }
        if self.shcsr_ena & (1 << 16) != 0 && ep > self.group(self.prio[EXC_MEMMANAGE] as i32) {
            v |= 1 << 5;
        }
        if self.shcsr_ena & (1 << 17) != 0 && ep > self.group(self.prio[EXC_BUSFAULT] as i32) {
            v |= 1 << 6;
        }
        if self.demcr & (1 << 16) != 0 && ep > self.group(self.prio[EXC_DEBUGMON] as i32) {
            v |= 1 << 8;
        }
        self.fpccr = v;
        self.fpcar = (frameptr + 0x20) & !7;
    }

    fn exception_taken<B: Bus>(&mut self, bus: &mut B, n: usize) {
        let vaddr = self.vtor.wrapping_add(4 * n as u32);
        let vector = match bus.read(self.now, vaddr, 4, false) {
            Ok(v) => v,
            Err(_) => {
                // vector table read error: HardFault
                self.hfsr |= HFSR_VECTTBL;
                if n == EXC_HARDFAULT || self.execution_priority() <= -1 {
                    self.enter_lockup();
                    return;
                }
                return self.exception_taken(bus, EXC_HARDFAULT);
            }
        };
        self.pc = vector & !1;
        self.t = vector & 1 == 1;
        self.it = 0;
        self.set_stack_state(n as u32, false);
        self.fpca = false;
        self.active |= 1 << n;
        self.pending &= !(1 << n);
        self.excl = false;
        self.event_reg = true;
        self.exc_dirty = true;
        self.exceptions_taken += 1;
        self.extra_cycles += 12;
        if self.c_debugen {
            let vc = match n {
                EXC_HARDFAULT => self.demcr & (1 << 10) != 0,
                EXC_BUSFAULT => self.demcr & (1 << 8) != 0,
                EXC_MEMMANAGE => self.demcr & (1 << 4) != 0,
                EXC_USAGEFAULT => {
                    (self.cfsr & (CFSR_INVSTATE | CFSR_INVPC | CFSR_UNDEFINSTR) != 0 && self.demcr & (1 << 7) != 0)
                        || (self.cfsr & (CFSR_UNALIGNED | CFSR_DIVBYZERO) != 0 && self.demcr & (1 << 6) != 0)
                        || (self.cfsr & CFSR_NOCP != 0 && self.demcr & (1 << 5) != 0)
                }
                _ => false,
            };
            if vc {
                self.halted = Some(HaltReason::VectorCatch);
                self.dfsr |= DFSR_VCATCH;
            }
        }
    }

    /// Makes `exc` pending when it is enabled and may preempt, otherwise escalates to HardFault.
    fn escalate_or_pend(&mut self, exc: usize, enabled: bool) {
        let ep = self.execution_priority();
        if enabled && self.group(self.prio[exc] as i32) < ep {
            self.set_pending(exc);
        } else if ep > -1 {
            self.hfsr |= HFSR_FORCED;
            self.set_pending(EXC_HARDFAULT);
        } else {
            self.enter_lockup();
        }
    }

    fn enter_lockup(&mut self) {
        log::warn!("core lockup at pc {:#010x}", self.pc);
        self.lockup = true;
        self.pc = 0xEFFF_FFFE;
        self.reset_request = Some(ResetRequest::Lockup);
    }

    /// Handles a synchronous fault of the instruction at `ret`.
    pub(crate) fn raise_fault<B: Bus>(&mut self, bus: &mut B, trap: Trap, ret: u32) {
        let (exc, enabled) = match trap {
            Trap::Usage(bit) => {
                self.cfsr |= bit;
                (EXC_USAGEFAULT, self.shcsr_ena & (1 << 18) != 0)
            }
            Trap::Bus(bits, addr) => {
                self.cfsr |= bits;
                if let Some(a) = addr {
                    self.bfar = a;
                    self.cfsr |= CFSR_BFARVALID;
                }
                (EXC_BUSFAULT, self.shcsr_ena & (1 << 17) != 0)
            }
            Trap::Mem(bits, addr) => {
                self.cfsr |= bits;
                if let Some(a) = addr {
                    self.mmfar = a;
                    self.cfsr |= CFSR_MMARVALID;
                }
                (EXC_MEMMANAGE, self.shcsr_ena & (1 << 16) != 0)
            }
            Trap::Bkpt(_) => {
                self.hfsr |= HFSR_DEBUGEVT;
                (EXC_HARDFAULT, true)
            }
            Trap::Svc => (EXC_SVCALL, true),
        };
        let ep = self.execution_priority();
        let target = if exc != EXC_HARDFAULT && enabled && self.group(self.exc_priority(exc)) < ep {
            exc
        } else {
            if exc != EXC_HARDFAULT {
                self.hfsr |= HFSR_FORCED;
            }
            EXC_HARDFAULT
        };
        if target == EXC_HARDFAULT && ep <= -1 {
            self.enter_lockup();
            return;
        }
        self.take_exception(bus, target, ret);
    }

    /// `ExceptionReturn(EXC_RETURN)`.
    pub(crate) fn exception_return<B: Bus>(&mut self, bus: &mut B, exc_return: u32) {
        let returning = (self.ipsr & 0x1FF) as usize;
        let nested = self.active.count_ones();
        let fault_invpc = |cpu: &mut Self, bus: &mut B| {
            // UsageFault INVPC; the return address is the EXC_RETURN value as on hardware
            cpu.raise_fault(bus, Trap::Usage(CFSR_INVPC), exc_return);
        };
        if self.active & (1 << returning) == 0 || exc_return & 0x0FFF_FFE0 != 0x0FFF_FFE0 {
            return fault_invpc(self, bus);
        }
        let (to_thread, use_psp) = match exc_return & 0xF {
            0b0001 => {
                if nested == 1 {
                    return fault_invpc(self, bus);
                }
                (false, false)
            }
            0b1001 | 0b1101 => {
                if nested != 1 && self.ccr & 1 == 0 {
                    return fault_invpc(self, bus);
                }
                (true, exc_return & 0xF == 0b1101)
            }
            _ => return fault_invpc(self, bus),
        };
        // DeActivate
        self.active &= !(1 << returning);
        if returning != EXC_NMI {
            self.faultmask = false;
        }
        // level-sensitive IRQ still asserted: pending again
        if returning >= 16 && self.irq_lines & (1 << (returning - 16)) != 0 {
            self.pending |= 1 << returning;
        }
        // select the frame
        let frameptr = if use_psp { self.psp() } else { self.msp() };
        // PopStack
        let fp_frame = exc_return & 0x10 == 0;
        let framesize: u32 = if fp_frame { 0x68 } else { 0x20 };
        let forcealign = fp_frame || self.ccr & 0x200 != 0;
        let mut vals = [0u32; 8];
        let mut unstk_err = false;
        for (i, v) in vals.iter_mut().enumerate() {
            match bus.read(self.now, frameptr.wrapping_add(4 * i as u32), 4, false) {
                Ok(x) => *v = x,
                Err(_) => unstk_err = true,
            }
        }
        if fp_frame {
            if self.fpccr & 1 != 0 {
                self.fpccr &= !1;
            } else {
                for i in 0..16 {
                    if let Ok(x) = bus.read(self.now, frameptr + 0x20 + 4 * i, 4, false) {
                        self.s[i as usize] = x;
                    }
                }
                if let Ok(x) = bus.read(self.now, frameptr + 0x60, 4, false) {
                    self.fpscr = x & crate::fpu::FPSCR_MASK;
                }
            }
        }
        let psr = vals[7];
        let spmask = if (psr >> 9) & 1 == 1 && forcealign { 4 } else { 0 };
        let new_sp = frameptr.wrapping_add(framesize) | spmask;
        let new_ipsr = psr & 0x1FF;
        self.set_stack_state(if to_thread { 0 } else { self.ipsr }, if to_thread { use_psp } else { false });
        if use_psp {
            self.set_psp(new_sp);
        } else {
            self.set_msp(new_sp);
        }
        self.r[0] = vals[0];
        self.r[1] = vals[1];
        self.r[2] = vals[2];
        self.r[3] = vals[3];
        self.r[12] = vals[4];
        self.r[14] = vals[5];
        self.pc = vals[6] & !1;
        self.fpca = fp_frame;
        self.set_apsr_nzcvq(psr);
        self.ge = ((psr >> 16) & 0xF) as u8;
        self.t = (psr >> 24) & 1 != 0;
        self.it = ((((psr >> 10) & 0x3F) << 2) | ((psr >> 25) & 3)) as u8;
        let spsel = self.spsel;
        self.set_stack_state(new_ipsr, spsel);
        self.excl = false;
        self.event_reg = true;
        self.exc_dirty = true;
        self.extra_cycles += 10;
        if unstk_err {
            self.cfsr |= CFSR_UNSTKERR;
            self.escalate_or_pend(EXC_BUSFAULT, self.shcsr_ena & (1 << 17) != 0);
        }
        if (to_thread && new_ipsr != 0) || (!to_thread && new_ipsr == 0) {
            self.raise_fault(bus, Trap::Usage(CFSR_INVPC), self.pc);
            return;
        }
        if to_thread && self.active == 0 && self.scr & 2 != 0 {
            // SLEEPONEXIT
            self.sleeping = true;
            self.sleep_wfe = false;
        }
    }

    fn write_mem_stack<B: Bus>(&mut self, bus: &mut B, addr: u32, v: u32) -> Result<(), BusError> {
        if (0xE000_0000..0xE010_0000).contains(&addr) {
            return Err(BusError);
        }
        bus.write(self.now, addr, 4, v, false)
    }

    // ---------------------------------------------------------------- run loop

    /// Wake-up condition of WFI/WFE: an exception that would preempt if PRIMASK were clear.
    fn wake_condition(&self) -> bool {
        if self.sleep_wfe && self.event_reg {
            return true;
        }
        if self.compute_ready(false).is_some() {
            return true;
        }
        if self.scr & (1 << 4) != 0 && self.sleep_wfe && self.pending != 0 {
            return true;
        }
        false
    }

    /// Resumes execution after a debug halt.
    pub fn resume(&mut self) {
        self.halted = None;
        self.c_halt = false;
        self.skip_bp = true;
    }

    /// Requests a halt (C_HALT); takes effect at the next instruction boundary.
    pub fn request_halt(&mut self) {
        self.c_halt = true;
    }

    fn halt(&mut self, reason: HaltReason) -> Exit {
        self.halted = Some(reason);
        self.c_halt = true;
        self.sleeping = false;
        self.dfsr |= match reason {
            HaltReason::Bkpt(_) | HaltReason::Breakpoint => DFSR_BKPT,
            HaltReason::Watchpoint(..) => DFSR_DWTTRAP,
            HaltReason::VectorCatch => DFSR_VCATCH,
            _ => DFSR_HALTED,
        };
        Exit::Halted(reason)
    }

    /// Runs until `limit`, the bus deadline, a debug halt, sleep or a reset request.
    pub fn run<B: Bus>(&mut self, bus: &mut B, limit: Ticks) -> Exit {
        loop {
            if let Some(lines) = bus.take_irq_lines() {
                self.set_irq_lines(lines);
            }
            if let Some(r) = self.halted {
                return Exit::Halted(r);
            }
            if let Some(r) = self.reset_request.take() {
                return Exit::Reset(r);
            }
            if self.c_halt && self.c_debugen {
                return self.halt(HaltReason::Request);
            }
            if self.now >= self.systick.next_wrap {
                self.systick_wrap();
            }
            if let Some(n) = self.ready_exception() {
                self.sleeping = false;
                let ret = self.return_address_frame();
                self.take_exception(bus, n, ret);
                self.now += (self.extra_cycles as u64) * TICKS_PER_CPU_CYCLE;
                self.extra_cycles = 0;
                continue;
            }
            if self.sleeping {
                if self.wake_condition() {
                    self.sleeping = false;
                    if self.sleep_wfe {
                        self.event_reg = false;
                    }
                    continue;
                }
                return Exit::Sleeping;
            }
            let deadline = limit.min(bus.deadline()).min(self.systick.next_wrap);
            if self.now >= deadline {
                if self.now >= self.systick.next_wrap {
                    continue;
                }
                return Exit::Deadline;
            }
            if !std::mem::take(&mut self.skip_bp) && self.c_debugen && self.breakpoint_at(self.pc) {
                return self.halt(HaltReason::Breakpoint);
            }
            let step = self.step_request || (self.c_step && self.c_debugen);
            self.step_instruction(bus);
            if let Some(r) = self.halted {
                return Exit::Halted(r);
            }
            if let Some((a, k)) = self.watch_hit.take() {
                if self.c_debugen {
                    return self.halt(HaltReason::Watchpoint(a, k));
                }
            }
            if step {
                self.step_request = false;
                // a step that ends in sleep or an exception entry halts at the next instruction
                return self.halt(HaltReason::Step);
            }
        }
    }

    fn breakpoint_at(&self, pc: u32) -> bool {
        self.sw_breakpoints.contains(&pc) || self.fpb.matches(pc)
    }

    /// Fetches, decodes and executes one instruction.
    pub(crate) fn step_instruction<B: Bus>(&mut self, bus: &mut B) {
        let pc = self.pc;
        if !self.t {
            self.raise_fault(bus, Trap::Usage(CFSR_INVSTATE), pc);
            self.finish_cycles(1);
            return;
        }
        if let Err(t) = self.check_exec(pc) {
            self.raise_fault(bus, t, pc);
            self.finish_cycles(1);
            return;
        }
        let (ins, size) = match self.fetch_decode(bus, pc) {
            Ok(v) => v,
            Err(()) => {
                self.raise_fault(bus, Trap::Bus(CFSR_IBUSERR, None), pc);
                self.finish_cycles(1);
                return;
            }
        };
        self.extra_cycles += bus.fetch_wait_cycles(pc);
        self.next_pc = pc.wrapping_add(size);
        let is_it = matches!(ins, Instr::It { .. });
        let skip = self.it & 0xF != 0 && !is_it && !self.it_condition_passed();
        let cycles = if skip {
            1
        } else {
            match self.execute(bus, ins) {
                Ok(c) => c,
                Err(Trap::Svc) => {
                    self.it_advance();
                    self.pc = self.next_pc;
                    self.instret += 1;
                    let ret = self.pc;
                    self.raise_fault(bus, Trap::Svc, ret);
                    self.finish_cycles(1);
                    return;
                }
                Err(Trap::Bkpt(imm)) => {
                    if self.c_debugen {
                        self.halt(HaltReason::Bkpt(imm));
                    } else {
                        self.raise_fault(bus, Trap::Bkpt(imm), pc);
                    }
                    self.finish_cycles(1);
                    return;
                }
                Err(t) => {
                    self.raise_fault(bus, t, pc);
                    self.finish_cycles(1);
                    return;
                }
            }
        };
        if !is_it {
            self.it_advance();
        }
        self.pc = self.next_pc;
        self.instret += 1;
        self.finish_cycles(cycles);
        if let Some(er) = self.exc_return.take() {
            self.exception_return(bus, er);
            self.finish_cycles(0);
        }
    }

    /// Fetches and decodes the instruction at `pc`, from the decode cache when possible.
    #[inline]
    fn fetch_decode<B: Bus>(&mut self, bus: &mut B, pc: u32) -> Result<(Instr, u32), ()> {
        let limit = bus.code_cache_limit();
        if pc < limit {
            let gen = bus.code_generation();
            if gen != self.icache_gen || self.icache.len() != (limit / 2) as usize {
                self.icache = vec![Instr::Undefined; (limit / 2) as usize];
                self.icache_size = vec![0; (limit / 2) as usize];
                self.icache_gen = gen;
            }
            let i = (pc >> 1) as usize;
            let sz = self.icache_size[i];
            if sz != 0 {
                return Ok((self.icache[i], sz as u32));
            }
            let r = Self::decode_at(bus, pc)?;
            self.icache[i] = r.0;
            self.icache_size[i] = r.1 as u8;
            return Ok(r);
        }
        Self::decode_at(bus, pc)
    }

    fn decode_at<B: Bus>(bus: &mut B, pc: u32) -> Result<(Instr, u32), ()> {
        let hw1 = bus.fetch16(pc).map_err(|_| ())?;
        if is_32bit(hw1) {
            let hw2 = bus.fetch16(pc.wrapping_add(2)).map_err(|_| ())?;
            Ok((decode(hw1, hw2), 4))
        } else {
            Ok((decode(hw1, 0), 2))
        }
    }

    #[inline]
    fn finish_cycles(&mut self, cycles: u32) {
        self.now += ((cycles + self.extra_cycles) as u64) * TICKS_PER_CPU_CYCLE;
        self.extra_cycles = 0;
    }

    #[inline]
    fn it_condition_passed(&self) -> bool {
        crate::alu::condition_passed(self.it >> 4, self.n, self.z, self.c, self.v)
    }

    #[inline]
    pub(crate) fn in_it_block(&self) -> bool {
        self.it & 0xF != 0
    }

    #[inline]
    fn it_advance(&mut self) {
        if self.it & 0x7 == 0 {
            self.it = 0;
        } else {
            self.it = (self.it & 0xE0) | ((self.it << 1) & 0x1F);
        }
    }

    /// Execute-never check of the default memory map and the MPU.
    fn check_exec(&self, pc: u32) -> Result<(), Trap> {
        let xn_default = matches!(pc >> 28, 0x4 | 0x5 | 0xA..=0xF);
        if self.mpu.enabled() && !(self.handler_mode() && self.mpu.ctrl & 2 == 0 && self.execution_priority() < 0) {
            match self.mpu.check(pc, self.privileged(), false, true) {
                Some(true) => return Ok(()),
                Some(false) => return Err(Trap::Mem(CFSR_IACCVIOL, None)),
                None => {
                    if !(self.mpu.ctrl & 4 != 0 && self.privileged()) {
                        return Err(Trap::Mem(CFSR_IACCVIOL, None));
                    }
                }
            }
        }
        if xn_default {
            return Err(Trap::Mem(CFSR_IACCVIOL, None));
        }
        Ok(())
    }

    // ---------------------------------------------------------------- SysTick

    pub(crate) fn systick_wrap(&mut self) {
        let t = self.systick.next_wrap;
        self.systick.countflag = true;
        if self.systick.ctrl & 2 != 0 {
            self.set_pending(EXC_SYSTICK);
        }
        self.systick.after_wrap(t);
    }

    /// Advances an idle core to `t` (rounded up to a CPU clock edge).
    pub fn advance_to(&mut self, t: Ticks) {
        if t > self.now {
            self.now = ceil_cpu_cycle(t);
        }
    }

    /// Next time the core has its own event (SysTick wrap).
    pub fn next_internal_event(&self) -> Ticks {
        self.systick.next_wrap
    }
}
