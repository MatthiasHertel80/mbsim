//! The private peripheral bus of the Cortex-M4: ITM, DWT, FPB, the System Control Space
//! (NVIC, SCB, SysTick, MPU, FPU control, debug control), TPIU and the ROM table.

use crate::cpu::*;
use crate::fpu::FPSCR_MASK;
use mbsim_core::time::TICKS_PER_CPU_CYCLE;
use mbsim_core::Ticks;

/// SysTick timer, modelled analytically from the time of the next wrap.
#[derive(Clone, Debug)]
pub struct SysTick {
    pub ctrl: u32,
    pub load: u32,
    pub countflag: bool,
    /// Time at which the counter next reaches zero (u64::MAX when stopped).
    pub next_wrap: Ticks,
    /// Counter value while stopped.
    stopped_val: u32,
    /// False until the first wrap after an enable from a non-zero value.
    periodic: bool,
    pub calib: u32,
}

impl Default for SysTick {
    fn default() -> Self {
        SysTick {
            ctrl: 0,
            load: 0,
            countflag: false,
            next_wrap: Ticks::MAX,
            stopped_val: 0,
            periodic: true,
            // NOREF and SKEW; the value of the nRF52833 is to be captured from a board
            calib: 0xC000_0000,
        }
    }
}

impl SysTick {
    fn enabled(&self) -> bool {
        self.ctrl & 1 != 0
    }

    pub fn value(&self, now: Ticks) -> u32 {
        if !self.enabled() || self.next_wrap == Ticks::MAX {
            return self.stopped_val;
        }
        if now >= self.next_wrap {
            return 0;
        }
        let r = (self.next_wrap - now) / TICKS_PER_CPU_CYCLE;
        if self.periodic {
            (r % (self.load as u64 + 1)) as u32
        } else {
            r as u32
        }
    }

    fn start_from(&mut self, now: Ticks, val: u32) {
        if val == 0 {
            self.periodic = true;
            self.next_wrap = if self.load == 0 { Ticks::MAX } else { now + (self.load as u64 + 1) * TICKS_PER_CPU_CYCLE };
            if self.load == 0 {
                self.stopped_val = 0;
            }
        } else {
            self.periodic = false;
            self.next_wrap = now + val as u64 * TICKS_PER_CPU_CYCLE;
        }
    }

    pub fn after_wrap(&mut self, t: Ticks) {
        self.periodic = true;
        if self.load == 0 {
            self.next_wrap = Ticks::MAX;
            self.stopped_val = 0;
        } else {
            self.next_wrap = t + (self.load as u64 + 1) * TICKS_PER_CPU_CYCLE;
        }
    }

    fn write_ctrl(&mut self, now: Ticks, v: u32) {
        let was = self.enabled();
        let cur = self.value(now);
        self.ctrl = v & 7;
        match (was, self.enabled()) {
            (false, true) => self.start_from(now, self.stopped_val),
            (true, false) => {
                self.stopped_val = cur;
                self.next_wrap = Ticks::MAX;
            }
            _ => {}
        }
    }

    fn write_val(&mut self, now: Ticks) {
        self.countflag = false;
        self.stopped_val = 0;
        if self.enabled() {
            self.start_from(now, 0);
        }
    }
}

/// PMSAv7 MPU with 8 regions.
#[derive(Clone, Debug, Default)]
pub struct Mpu {
    pub ctrl: u32,
    pub rnr: u32,
    pub rbar: [u32; 8],
    pub rasr: [u32; 8],
}

impl Mpu {
    pub fn enabled(&self) -> bool {
        self.ctrl & 1 != 0
    }

    /// Permission check: `None` when no region matches, otherwise whether the access is allowed.
    pub fn check(&self, addr: u32, privileged: bool, write: bool, exec: bool) -> Option<bool> {
        for i in (0..8).rev() {
            let rasr = self.rasr[i];
            if rasr & 1 == 0 {
                continue;
            }
            let size_field = (rasr >> 1) & 0x1F;
            if size_field < 4 {
                continue;
            }
            let size = 1u64 << (size_field + 1);
            let base = (self.rbar[i] as u64) & !(size - 1) & 0xFFFF_FFE0;
            let a = addr as u64;
            if a < base || a >= base + size {
                continue;
            }
            if size >= 256 {
                let sub = ((a - base) / (size / 8)) as u32;
                if (rasr >> 8) & (1 << sub) != 0 {
                    continue;
                }
            }
            let ap = (rasr >> 24) & 7;
            let (pr, pw, ur, uw) = match ap {
                0b001 => (true, true, false, false),
                0b010 => (true, true, true, false),
                0b011 => (true, true, true, true),
                0b101 => (true, false, false, false),
                0b110 | 0b111 => (true, false, true, false),
                _ => (false, false, false, false),
            };
            let ok = if exec {
                rasr & (1 << 28) == 0 && if privileged { pr } else { ur }
            } else if write {
                if privileged {
                    pw
                } else {
                    uw
                }
            } else if privileged {
                pr
            } else {
                ur
            };
            return Some(ok);
        }
        None
    }
}

/// Data watchpoint and trace unit with 4 comparators.
#[derive(Clone, Debug)]
pub struct Dwt {
    pub ctrl: u32,
    cyc_base: u32,
    cyc_origin: Ticks,
    cyc_running: bool,
    pub cpicnt: u32,
    pub exccnt: u32,
    pub sleepcnt: u32,
    pub lsucnt: u32,
    pub foldcnt: u32,
    pub comp: [u32; 4],
    pub mask: [u32; 4],
    pub function: [u32; 4],
}

impl Default for Dwt {
    fn default() -> Self {
        Dwt {
            ctrl: 0x4000_0000,
            cyc_base: 0,
            cyc_origin: 0,
            cyc_running: false,
            cpicnt: 0,
            exccnt: 0,
            sleepcnt: 0,
            lsucnt: 0,
            foldcnt: 0,
            comp: [0; 4],
            mask: [0; 4],
            function: [0; 4],
        }
    }
}

impl Dwt {
    pub fn cyccnt(&self, now: Ticks) -> u32 {
        if self.cyc_running {
            self.cyc_base.wrapping_add(((now - self.cyc_origin) / TICKS_PER_CPU_CYCLE) as u32)
        } else {
            self.cyc_base
        }
    }

    fn set_cyccnt(&mut self, now: Ticks, v: u32) {
        self.cyc_base = v;
        self.cyc_origin = now;
    }

    /// Re-evaluates whether CYCCNT counts (CYCCNTENA and DEMCR.TRCENA).
    pub fn update_running(&mut self, now: Ticks, trcena: bool) {
        let run = self.ctrl & 1 != 0 && trcena;
        if run != self.cyc_running {
            let v = self.cyccnt(now);
            self.cyc_running = run;
            self.set_cyccnt(now, v);
        }
    }

    /// Data address comparators that match an access; returns the matching address.
    pub fn match_data(&mut self, addr: u32, size: u32, write: bool) -> Option<u32> {
        let mut hit = None;
        for i in 0..4 {
            let f = self.function[i] & 0xF;
            let kind_ok = match f {
                5 => !write,
                6 => write,
                7 => true,
                _ => false,
            };
            if !kind_ok {
                continue;
            }
            let m = self.mask[i] & 0x1F;
            let lo = self.comp[i] & !((1u64 << m) - 1) as u32;
            let hi = lo as u64 + (1u64 << m);
            if (addr as u64) < hi && (addr as u64 + size as u64) > lo as u64 {
                self.function[i] |= 1 << 24;
                hit = Some(addr);
            }
        }
        hit
    }
}

/// Flash patch and breakpoint unit (FPBv1: 6 code and 2 literal comparators).
#[derive(Clone, Debug, Default)]
pub struct Fpb {
    pub ctrl: u32,
    pub comp: [u32; 8],
}

impl Fpb {
    pub fn matches(&self, pc: u32) -> bool {
        if self.ctrl & 1 == 0 || pc >= 0x2000_0000 {
            return false;
        }
        self.comp[..6].iter().any(|&c| {
            if c & 1 == 0 || (c & 0x1FFF_FFFC) != (pc & 0x1FFF_FFFC) {
                return false;
            }
            match c >> 30 {
                1 => pc & 2 == 0,
                2 => pc & 2 != 0,
                3 => true,
                _ => false,
            }
        })
    }

    /// Free code comparator index.
    pub fn free_comparator(&self) -> Option<usize> {
        self.comp[..6].iter().position(|c| c & 1 == 0)
    }
}

/// Instrumentation trace macrocell; stimulus writes are collected for the host.
#[derive(Clone, Debug, Default)]
pub struct Itm {
    pub ter: u32,
    pub tpr: u32,
    pub tcr: u32,
    pub out: Vec<(u8, u32, u8)>,
}

const SCS_PIDR: [u32; 12] = [0x04, 0, 0, 0, 0x0C, 0xB0, 0x0B, 0x00, 0x0D, 0xE0, 0x05, 0xB1];
const DWT_PIDR: [u32; 12] = [0x04, 0, 0, 0, 0x02, 0xB0, 0x3B, 0x00, 0x0D, 0xE0, 0x05, 0xB1];
const FPB_PIDR: [u32; 12] = [0x04, 0, 0, 0, 0x03, 0xB0, 0x2B, 0x00, 0x0D, 0xE0, 0x05, 0xB1];
const ITM_PIDR: [u32; 12] = [0x04, 0, 0, 0, 0x01, 0xB0, 0x3B, 0x00, 0x0D, 0xE0, 0x05, 0xB1];
const TPIU_PIDR: [u32; 12] = [0x04, 0, 0, 0, 0xA1, 0xB9, 0x0B, 0x00, 0x0D, 0x90, 0x05, 0xB1];
const ROM_PIDR: [u32; 12] = [0x04, 0, 0, 0, 0xC4, 0xB4, 0x0B, 0x00, 0x0D, 0x10, 0x05, 0xB1];

fn id_reg(table: &[u32; 12], off: u32) -> u32 {
    if (0xFD0..=0xFFC).contains(&off) {
        table[((off - 0xFD0) / 4) as usize]
    } else {
        0
    }
}

fn bus_fault(addr: u32) -> Trap {
    Trap::Bus(CFSR_PRECISERR, Some(addr))
}

impl Cpu {
    /// Read of the private peripheral bus. `debug` reads have no side effects.
    pub fn ppb_read(&mut self, addr: u32, size: u8, debug: bool) -> Result<u32, Trap> {
        let word_addr = addr & !3;
        if !debug && !self.privileged() && !(0xE000_0000..0xE000_1000).contains(&addr) {
            return Err(bus_fault(addr));
        }
        let w = self.ppb_read_word(word_addr, debug)?;
        let shift = (addr & 3) * 8;
        Ok(match size {
            1 => (w >> shift) & 0xFF,
            2 => (w >> shift) & 0xFFFF,
            _ => w,
        })
    }

    /// Write to the private peripheral bus.
    pub fn ppb_write(&mut self, addr: u32, size: u8, value: u32, debug: bool) -> Result<(), Trap> {
        let word_addr = addr & !3;
        let unpriv_ok = (0xE000_0000..0xE000_1000).contains(&addr) || (addr == 0xE000_EF00 && self.ccr & 2 != 0);
        if !debug && !self.privileged() && !unpriv_ok {
            return Err(bus_fault(addr));
        }
        let shift = (addr & 3) * 8;
        let (val, mask) = match size {
            1 => ((value & 0xFF) << shift, 0xFFu32 << shift),
            2 => ((value & 0xFFFF) << shift, 0xFFFFu32 << shift),
            _ => (value, 0xFFFF_FFFF),
        };
        self.ppb_write_word(word_addr, val, mask, debug)
    }

    fn ppb_read_word(&mut self, a: u32, debug: bool) -> Result<u32, Trap> {
        let now = self.now;
        Ok(match a {
            // ITM
            0xE000_0000..=0xE000_007C => 1, // stimulus port FIFO ready
            0xE000_0E00 => self.itm.ter,
            0xE000_0E40 => self.itm.tpr,
            0xE000_0E80 => self.itm.tcr,
            0xE000_0FB4 => 0,
            0xE000_0FD0..=0xE000_0FFC => id_reg(&ITM_PIDR, a - 0xE000_0000),
            // DWT
            0xE000_1000 => self.dwt.ctrl,
            0xE000_1004 => self.dwt.cyccnt(now),
            0xE000_1008 => self.dwt.cpicnt,
            0xE000_100C => self.dwt.exccnt,
            0xE000_1010 => self.dwt.sleepcnt,
            0xE000_1014 => self.dwt.lsucnt,
            0xE000_1018 => self.dwt.foldcnt,
            0xE000_101C => self.pc,
            0xE000_1020..=0xE000_105C => {
                let i = ((a - 0xE000_1020) / 16) as usize;
                match (a - 0xE000_1020) % 16 {
                    0 => self.dwt.comp[i],
                    4 => self.dwt.mask[i],
                    8 => {
                        let f = self.dwt.function[i];
                        if !debug {
                            self.dwt.function[i] &= !(1 << 24);
                        }
                        f
                    }
                    _ => 0,
                }
            }
            0xE000_1FD0..=0xE000_1FFC => id_reg(&DWT_PIDR, a - 0xE000_1000),
            // FPB
            0xE000_2000 => self.fpb.ctrl | (0x6 << 4) | (0x2 << 8) | 0x2,
            0xE000_2004 => 0,
            0xE000_2008..=0xE000_2024 => self.fpb.comp[((a - 0xE000_2008) / 4) as usize],
            0xE000_2FD0..=0xE000_2FFC => id_reg(&FPB_PIDR, a - 0xE000_2000),
            // SCS
            0xE000_E004 => (NUM_IRQ as u32).div_ceil(32) - 1,
            0xE000_E008 => self.actlr,
            0xE000_E010 => {
                let v = self.systick.ctrl | ((self.systick.countflag as u32) << 16);
                if !debug {
                    self.systick.countflag = false;
                }
                v
            }
            0xE000_E014 => self.systick.load,
            0xE000_E018 => self.systick.value(now),
            0xE000_E01C => self.systick.calib,
            0xE000_E100..=0xE000_E13C => self.irq_word(a - 0xE000_E100, self.irq_enabled),
            0xE000_E180..=0xE000_E1BC => self.irq_word(a - 0xE000_E180, self.irq_enabled),
            0xE000_E200..=0xE000_E23C => self.irq_word(a - 0xE000_E200, self.pending >> 16),
            0xE000_E280..=0xE000_E2BC => self.irq_word(a - 0xE000_E280, self.pending >> 16),
            0xE000_E300..=0xE000_E33C => self.irq_word(a - 0xE000_E300, self.active >> 16),
            0xE000_E400..=0xE000_E5EC => {
                let base = (a - 0xE000_E400) as usize;
                let mut v = 0;
                for i in 0..4 {
                    let irq = base + i;
                    if irq < NUM_IRQ {
                        v |= (self.prio[16 + irq] as u32) << (8 * i);
                    }
                }
                v
            }
            0xE000_ED00 => 0x410F_C241,
            0xE000_ED04 => self.icsr(),
            0xE000_ED08 => self.vtor,
            0xE000_ED0C => 0xFA05_0000 | (self.prigroup << 8),
            0xE000_ED10 => self.scr,
            0xE000_ED14 => self.ccr,
            0xE000_ED18..=0xE000_ED20 => {
                let base = 4 + (a - 0xE000_ED18) as usize;
                (0..4).fold(0, |v, i| v | ((self.prio[base + i] as u32) << (8 * i)))
            }
            0xE000_ED24 => self.shcsr(),
            0xE000_ED28 => self.cfsr,
            0xE000_ED2C => self.hfsr,
            0xE000_ED30 => self.dfsr,
            0xE000_ED34 => self.mmfar,
            0xE000_ED38 => self.bfar,
            0xE000_ED3C => self.afsr,
            0xE000_ED40 => 0x0000_0030,
            0xE000_ED44 => 0x0000_0200,
            0xE000_ED48 => 0x0010_0000,
            0xE000_ED4C => 0,
            0xE000_ED50 => 0x0010_0030,
            0xE000_ED54 => 0,
            0xE000_ED58 => 0x0100_0000,
            0xE000_ED5C => 0,
            0xE000_ED60 => 0x0110_1110,
            0xE000_ED64 => 0x0211_1000,
            0xE000_ED68 => 0x2111_2231,
            0xE000_ED6C => 0x0111_1110,
            0xE000_ED70 => 0x0131_0132,
            0xE000_ED88 => self.cpacr,
            0xE000_ED90 => 0x0000_0800,
            0xE000_ED94 => self.mpu.ctrl,
            0xE000_ED98 => self.mpu.rnr,
            0xE000_ED9C | 0xE000_EDA4 | 0xE000_EDAC | 0xE000_EDB4 => {
                let r = (self.mpu.rnr & 7) as usize;
                (self.mpu.rbar[r] & 0xFFFF_FFE0) | r as u32
            }
            0xE000_EDA0 | 0xE000_EDA8 | 0xE000_EDB0 | 0xE000_EDB8 => self.mpu.rasr[(self.mpu.rnr & 7) as usize],
            0xE000_EDF0 => self.dhcsr(),
            0xE000_EDF4 => 0,
            0xE000_EDF8 => self.dcrdr,
            0xE000_EDFC => self.demcr,
            0xE000_EF34 => self.fpccr,
            0xE000_EF38 => self.fpcar,
            0xE000_EF3C => self.fpdscr,
            0xE000_EF40 => 0x1011_0021,
            0xE000_EF44 => 0x1100_0011,
            0xE000_EFD0..=0xE000_EFFC => id_reg(&SCS_PIDR, a - 0xE000_E000),
            // TPIU
            0xE004_0000 => 0xF,
            0xE004_0004 => 1,
            0xE004_0FC8 => 0xCA0,
            0xE004_0FD0..=0xE004_0FFC => id_reg(&TPIU_PIDR, a - 0xE004_0000),
            // ROM table
            0xE00F_F000 => 0xFFF0_F003,
            0xE00F_F004 => 0xFFF0_2003,
            0xE00F_F008 => 0xFFF0_3003,
            0xE00F_F00C => 0xFFF0_1003,
            0xE00F_F010 => 0xFFF4_1003,
            0xE00F_F014 => 0xFFF4_2002,
            0xE00F_FFCC => 1,
            0xE00F_FFD0..=0xE00F_FFFC => id_reg(&ROM_PIDR, a - 0xE00F_F000),
            _ => 0,
        })
    }

    fn irq_word(&self, off: u32, bits: u64) -> u32 {
        let i = off / 4;
        if i < 2 {
            ((bits >> (32 * i)) & 0xFFFF_FFFF) as u32 & if i == 1 { (1 << (NUM_IRQ - 32)) - 1 } else { !0 }
        } else {
            0
        }
    }

    fn icsr(&self) -> u32 {
        let vectpending = self.vect_pending();
        let isrpending = (self.pending >> 16) & self.irq_enabled != 0 || self.irq_lines & self.irq_enabled != 0;
        let active_count = self.active.count_ones();
        let vectactive = self.ipsr & 0x1FF;
        let rettobase = vectactive != 0 && active_count == 1;
        ((self.pending >> EXC_NMI & 1) as u32) << 31
            | ((self.pending >> EXC_PENDSV & 1) as u32) << 28
            | ((self.pending >> EXC_SYSTICK & 1) as u32) << 26
            | (isrpending as u32) << 22
            | (vectpending & 0x1FF) << 12
            | (rettobase as u32) << 11
            | vectactive
    }

    fn shcsr(&self) -> u32 {
        let a = |n: usize| ((self.active >> n) & 1) as u32;
        let p = |n: usize| ((self.pending >> n) & 1) as u32;
        self.shcsr_ena
            | p(EXC_SVCALL) << 15
            | p(EXC_BUSFAULT) << 14
            | p(EXC_MEMMANAGE) << 13
            | p(EXC_USAGEFAULT) << 12
            | a(EXC_SYSTICK) << 11
            | a(EXC_PENDSV) << 10
            | a(EXC_DEBUGMON) << 8
            | a(EXC_SVCALL) << 7
            | a(EXC_USAGEFAULT) << 3
            | a(EXC_BUSFAULT) << 1
            | a(EXC_MEMMANAGE)
    }

    pub fn dhcsr(&self) -> u32 {
        ((self.s_reset_st as u32) << 25)
            | ((self.s_retire_st as u32) << 24)
            | ((self.lockup as u32) << 19)
            | ((self.sleeping as u32) << 18)
            | ((self.halted.is_some() as u32) << 17)
            | (1 << 16)
            | ((self.c_maskints as u32) << 3)
            | ((self.c_step as u32) << 2)
            | ((self.c_halt as u32) << 1)
            | self.c_debugen as u32
    }

    fn ppb_write_word(&mut self, a: u32, v: u32, mask: u32, debug: bool) -> Result<(), Trap> {
        let now = self.now;
        let full = mask == 0xFFFF_FFFF;
        match a {
            0xE000_0000..=0xE000_007C => {
                let port = ((a - 0xE000_0000) / 4) as u8;
                if self.itm.tcr & 1 != 0 && self.itm.ter & (1 << port) != 0 && self.demcr & (1 << 24) != 0 {
                    let size = mask.count_ones() / 8;
                    let shift = mask.trailing_zeros();
                    self.itm.out.push((port, v >> shift, size as u8));
                }
            }
            0xE000_0E00 => self.itm.ter = merge(self.itm.ter, v, mask),
            0xE000_0E40 => self.itm.tpr = merge(self.itm.tpr, v, mask) & 0xF,
            0xE000_0E80 => self.itm.tcr = merge(self.itm.tcr, v, mask) & 0x00FF_FF1F,
            0xE000_1000 => {
                self.dwt.ctrl = (self.dwt.ctrl & 0xF000_0000) | (merge(self.dwt.ctrl, v, mask) & 0x007F_1FFF);
                let trcena = self.demcr & (1 << 24) != 0;
                self.dwt.update_running(now, trcena);
            }
            0xE000_1004 => {
                let v = merge(self.dwt.cyccnt(now), v, mask);
                self.dwt.set_cyccnt(now, v);
            }
            0xE000_1008 => self.dwt.cpicnt = v & 0xFF,
            0xE000_100C => self.dwt.exccnt = v & 0xFF,
            0xE000_1010 => self.dwt.sleepcnt = v & 0xFF,
            0xE000_1014 => self.dwt.lsucnt = v & 0xFF,
            0xE000_1018 => self.dwt.foldcnt = v & 0xFF,
            0xE000_1020..=0xE000_105C => {
                let i = ((a - 0xE000_1020) / 16) as usize;
                match (a - 0xE000_1020) % 16 {
                    0 => self.dwt.comp[i] = merge(self.dwt.comp[i], v, mask),
                    4 => self.dwt.mask[i] = merge(self.dwt.mask[i], v, mask) & 0x1F,
                    8 => self.dwt.function[i] = (self.dwt.function[i] & (1 << 24)) | (merge(self.dwt.function[i], v, mask) & 0x000F_FDFF),
                    _ => {}
                }
            }
            0xE000_2000 if v & 2 != 0 => {
                self.fpb.ctrl = v & 1;
            }
            0xE000_2008..=0xE000_2024 => {
                let i = ((a - 0xE000_2008) / 4) as usize;
                self.fpb.comp[i] = merge(self.fpb.comp[i], v, mask) & 0xDFFF_FFFD;
            }
            0xE000_E008 => self.actlr = merge(self.actlr, v, mask) & 0x307,
            0xE000_E010 => {
                let nv = merge(self.systick.ctrl, v, mask);
                self.systick.write_ctrl(now, nv);
            }
            0xE000_E014 => self.systick.load = merge(self.systick.load, v, mask) & 0x00FF_FFFF,
            0xE000_E018 => self.systick.write_val(now),
            0xE000_E100..=0xE000_E104 => {
                let s = 32 * ((a - 0xE000_E100) / 4);
                self.irq_enabled |= ((v & mask) as u64) << s;
                self.irq_enabled &= (1 << NUM_IRQ) - 1;
                self.mark_exc_dirty();
            }
            0xE000_E180..=0xE000_E184 => {
                let s = 32 * ((a - 0xE000_E180) / 4);
                self.irq_enabled &= !(((v & mask) as u64) << s);
                self.mark_exc_dirty();
            }
            0xE000_E200..=0xE000_E204 => {
                let s = 32 * ((a - 0xE000_E200) / 4);
                let bits = (((v & mask) as u64) << s) & ((1 << NUM_IRQ) - 1);
                self.pending |= bits << 16;
                self.mark_exc_dirty();
            }
            0xE000_E280..=0xE000_E284 => {
                let s = 32 * ((a - 0xE000_E280) / 4);
                let mut bits = (((v & mask) as u64) << s) & ((1 << NUM_IRQ) - 1);
                while bits != 0 {
                    let i = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    self.clear_pending(16 + i);
                }
            }
            0xE000_E400..=0xE000_E42C => {
                let base = (a - 0xE000_E400) as usize;
                for i in 0..4 {
                    if mask & (0xFF << (8 * i)) != 0 && base + i < NUM_IRQ {
                        self.prio[16 + base + i] = ((v >> (8 * i)) & 0xE0) as u8;
                    }
                }
                self.mark_exc_dirty();
            }
            0xE000_ED04 => {
                let v = v & mask;
                if v & (1 << 31) != 0 {
                    self.set_pending(EXC_NMI);
                }
                if v & (1 << 28) != 0 {
                    self.set_pending(EXC_PENDSV);
                } else if v & (1 << 27) != 0 {
                    self.clear_pending(EXC_PENDSV);
                }
                if v & (1 << 26) != 0 {
                    self.set_pending(EXC_SYSTICK);
                } else if v & (1 << 25) != 0 {
                    self.clear_pending(EXC_SYSTICK);
                }
            }
            0xE000_ED08 => self.vtor = merge(self.vtor, v, mask) & 0xFFFF_FF80,
            0xE000_ED0C if full && v >> 16 == 0x05FA => {
                self.prigroup = (v >> 8) & 7;
                self.mark_exc_dirty();
                if v & 4 != 0 {
                    self.reset_request = Some(crate::bus::ResetRequest::System);
                }
                if v & 2 != 0 && self.halted.is_some() {
                    // VECTCLRACTIVE (debug only)
                    self.active = 0;
                    self.ipsr = 0;
                }
            }
            0xE000_ED10 => self.scr = merge(self.scr, v, mask) & 0x16,
            0xE000_ED14 => self.ccr = merge(self.ccr, v, mask) & 0x31B,
            0xE000_ED18..=0xE000_ED20 => {
                let base = 4 + (a - 0xE000_ED18) as usize;
                for i in 0..4 {
                    let n = base + i;
                    let configurable = matches!(n, 4..=6 | 11 | 12 | 14 | 15);
                    if mask & (0xFF << (8 * i)) != 0 && configurable {
                        self.prio[n] = ((v >> (8 * i)) & 0xE0) as u8;
                    }
                }
                self.mark_exc_dirty();
            }
            0xE000_ED24 => {
                let v = merge(self.shcsr(), v, mask);
                self.shcsr_ena = v & 0x0007_0000;
                let set = |cpu: &mut Self, n: usize, on: bool| {
                    if on {
                        cpu.pending |= 1 << n;
                    } else {
                        cpu.pending &= !(1 << n);
                    }
                };
                set(self, EXC_SVCALL, v & (1 << 15) != 0);
                set(self, EXC_BUSFAULT, v & (1 << 14) != 0);
                set(self, EXC_MEMMANAGE, v & (1 << 13) != 0);
                set(self, EXC_USAGEFAULT, v & (1 << 12) != 0);
                let act = |cpu: &mut Self, n: usize, on: bool| {
                    if on {
                        cpu.active |= 1 << n;
                    } else {
                        cpu.active &= !(1 << n);
                    }
                };
                act(self, EXC_SYSTICK, v & (1 << 11) != 0);
                act(self, EXC_PENDSV, v & (1 << 10) != 0);
                act(self, EXC_DEBUGMON, v & (1 << 8) != 0);
                act(self, EXC_SVCALL, v & (1 << 7) != 0);
                act(self, EXC_USAGEFAULT, v & (1 << 3) != 0);
                act(self, EXC_BUSFAULT, v & (1 << 1) != 0);
                act(self, EXC_MEMMANAGE, v & 1 != 0);
                self.mark_exc_dirty();
            }
            0xE000_ED28 => self.cfsr &= !(v & mask),
            0xE000_ED2C => self.hfsr &= !(v & mask),
            0xE000_ED30 => self.dfsr &= !(v & mask),
            0xE000_ED34 => self.mmfar = merge(self.mmfar, v, mask),
            0xE000_ED38 => self.bfar = merge(self.bfar, v, mask),
            0xE000_ED3C => self.afsr &= !(v & mask),
            0xE000_ED88 => self.cpacr = merge(self.cpacr, v, mask) & 0x00F0_0000,
            0xE000_ED94 => self.mpu.ctrl = merge(self.mpu.ctrl, v, mask) & 7,
            0xE000_ED98 => self.mpu.rnr = merge(self.mpu.rnr, v, mask) & 7,
            0xE000_ED9C | 0xE000_EDA4 | 0xE000_EDAC | 0xE000_EDB4 => {
                let v = merge(self.mpu.rbar[(self.mpu.rnr & 7) as usize], v, mask);
                if v & 0x10 != 0 {
                    self.mpu.rnr = v & 7;
                }
                self.mpu.rbar[(self.mpu.rnr & 7) as usize] = v & 0xFFFF_FFE0;
            }
            0xE000_EDA0 | 0xE000_EDA8 | 0xE000_EDB0 | 0xE000_EDB8 => {
                let r = (self.mpu.rnr & 7) as usize;
                self.mpu.rasr[r] = merge(self.mpu.rasr[r], v, mask) & 0x173F_FF3F;
            }
            0xE000_EDF0 => self.write_dhcsr(v, mask, debug),
            0xE000_EDF4 if debug && self.halted.is_some() => {
                let sel = v & 0x7F;
                if v & (1 << 16) != 0 {
                    let d = self.dcrdr;
                    self.write_core_reg(sel, d);
                } else {
                    self.dcrdr = self.read_core_reg(sel).unwrap_or(0);
                }
            }
            0xE000_EDF8 => self.dcrdr = merge(self.dcrdr, v, mask),
            0xE000_EDFC => {
                self.demcr = merge(self.demcr, v, mask) & 0x010F_07F1;
                let trcena = self.demcr & (1 << 24) != 0;
                self.dwt.update_running(now, trcena);
            }
            0xE000_EF00 => {
                let irq = (v & mask & 0x1FF) as usize;
                self.pend_irq(irq);
            }
            0xE000_EF34 => {
                let v = merge(self.fpccr, v, mask);
                self.fpccr = (v & 0xC000_0000) | (v & 0x17B);
            }
            0xE000_EF38 => self.fpcar = merge(self.fpcar, v, mask) & !7,
            0xE000_EF3C => self.fpdscr = merge(self.fpdscr, v, mask) & 0x07C0_0000,
            _ => {}
        }
        Ok(())
    }

    fn write_dhcsr(&mut self, v: u32, mask: u32, debug: bool) {
        if mask != 0xFFFF_FFFF || v >> 16 != 0xA05F {
            return;
        }
        if debug {
            self.c_debugen = v & 1 != 0;
        }
        if !self.c_debugen {
            return;
        }
        self.c_maskints = v & 8 != 0;
        self.c_step = v & 4 != 0;
        let halt = v & 2 != 0;
        if halt {
            self.c_halt = true;
        } else if self.halted.is_some() {
            self.resume();
        } else {
            self.c_halt = false;
        }
        self.mark_exc_dirty();
        if debug {
            // reading DHCSR clears the sticky S_RESET_ST and S_RETIRE_ST bits
            self.s_reset_st = false;
            self.s_retire_st = false;
        }
    }

    /// FPSCR write with the writable mask.
    pub fn set_fpscr(&mut self, v: u32) {
        self.fpscr = v & FPSCR_MASK;
    }
}

#[inline]
fn merge(old: u32, new: u32, mask: u32) -> u32 {
    (old & !mask) | (new & mask)
}
