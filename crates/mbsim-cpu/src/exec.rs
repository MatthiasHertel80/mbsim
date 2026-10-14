//! Instruction execution (DDI 0403E chapter A7 "Instruction Details").
//!
//! Cycle counts follow the Cortex-M4 TRM (DDI 0439) table 3-1 with a pipeline refill of two
//! cycles for taken branches.

use crate::alu::*;
use crate::bus::Bus;
use crate::cpu::*;
use crate::decode::*;
use crate::fpu;

const BRANCH_CYCLES: u32 = 3;

impl Cpu {
    #[inline]
    pub(crate) fn reg(&self, n: u8) -> u32 {
        if n == 15 {
            self.pc.wrapping_add(4)
        } else {
            self.r[n as usize]
        }
    }

    #[inline]
    pub(crate) fn set_reg(&mut self, n: u8, v: u32) {
        if n == 13 {
            self.r[13] = v & !3;
        } else {
            self.r[n as usize] = v;
        }
    }

    #[inline]
    fn sflag(&self, s: SetFlags) -> bool {
        match s {
            SetFlags::No => false,
            SetFlags::Yes => true,
            SetFlags::OutsideIt => !self.in_it_block(),
        }
    }

    /// `BXWritePC`/`LoadWritePC`: interworking branch, exception return in handler mode.
    #[inline]
    fn bx_write_pc(&mut self, v: u32) {
        if self.handler_mode() && v >> 28 == 0xF {
            self.exc_return = Some(v);
        } else {
            self.t = v & 1 != 0;
            self.next_pc = v & !1;
        }
    }

    fn check_watch(&mut self, addr: u32, size: u32, write: bool) {
        if !self.c_debugen {
            return;
        }
        if self.dwt.match_data(addr, size, write).is_some() {
            self.watch_hit = Some((addr, write as u8));
            return;
        }
        for w in &self.watchpoints {
            let kind_ok = if write { w.kind & 2 != 0 } else { w.kind & 1 != 0 };
            if kind_ok && addr < w.addr.wrapping_add(w.len) && addr.wrapping_add(size) > w.addr {
                self.watch_hit = Some((w.addr, write as u8));
                return;
            }
        }
    }

    fn check_data_mpu(&self, addr: u32, write: bool, unpriv: bool) -> Result<(), Trap> {
        if !self.mpu.enabled() {
            return Ok(());
        }
        if self.mpu.ctrl & 2 == 0 && self.execution_priority() < 0 {
            return Ok(());
        }
        if (0xE000_0000..0xE010_0000).contains(&addr) {
            return Ok(());
        }
        let privileged = self.privileged() && !unpriv;
        match self.mpu.check(addr, privileged, write, false) {
            Some(true) => Ok(()),
            Some(false) => Err(Trap::Mem(CFSR_DACCVIOL, Some(addr))),
            None => {
                if self.mpu.ctrl & 4 != 0 && privileged {
                    Ok(())
                } else {
                    Err(Trap::Mem(CFSR_DACCVIOL, Some(addr)))
                }
            }
        }
    }

    /// Data read. `aligned` requires natural alignment (MemA); otherwise unaligned word and
    /// halfword accesses are allowed unless CCR.UNALIGN_TRP is set (MemU).
    pub(crate) fn mem_read<B: Bus>(&mut self, bus: &mut B, addr: u32, size: u8, aligned: bool, unpriv: bool) -> Result<u32, Trap> {
        if size > 1 && addr & (size as u32 - 1) != 0 {
            if aligned || self.ccr & 8 != 0 {
                return Err(Trap::Usage(CFSR_UNALIGNED));
            }
            let mut v = 0;
            for i in 0..size as u32 {
                v |= self.mem_read(bus, addr.wrapping_add(i), 1, true, unpriv)? << (8 * i);
            }
            return Ok(v);
        }
        self.check_data_mpu(addr, false, unpriv)?;
        if self.watch_active && self.c_debugen {
            self.check_watch(addr, size as u32, false);
        }
        if addr >> 20 == 0xE00 {
            if unpriv {
                return Err(Trap::Bus(CFSR_PRECISERR, Some(addr)));
            }
            return self.ppb_read(addr, size, false);
        }
        bus.read(self.now, addr, size, false).map_err(|_| Trap::Bus(CFSR_PRECISERR, Some(addr)))
    }

    pub(crate) fn mem_write<B: Bus>(&mut self, bus: &mut B, addr: u32, size: u8, v: u32, aligned: bool, unpriv: bool) -> Result<(), Trap> {
        if size > 1 && addr & (size as u32 - 1) != 0 {
            if aligned || self.ccr & 8 != 0 {
                return Err(Trap::Usage(CFSR_UNALIGNED));
            }
            for i in 0..size as u32 {
                self.mem_write(bus, addr.wrapping_add(i), 1, (v >> (8 * i)) & 0xFF, true, unpriv)?;
            }
            return Ok(());
        }
        self.check_data_mpu(addr, true, unpriv)?;
        if self.watch_active && self.c_debugen {
            self.check_watch(addr, size as u32, true);
        }
        if addr >> 20 == 0xE00 {
            if unpriv && (addr >= 0xE000_1000) {
                return Err(Trap::Bus(CFSR_PRECISERR, Some(addr)));
            }
            let r = self.ppb_write(addr, size, v, false);
            if matches!(addr, 0xE000_1028 | 0xE000_1038 | 0xE000_1048 | 0xE000_1058) {
                self.refresh_watch();
            }
            return r;
        }
        let res = bus.write(self.now, addr, size, v, false);
        if let Some(t) = bus.take_stall() {
            self.now = self.now.max(mbsim_core::time::ceil_cpu_cycle(t));
        }
        if res.is_err() {
            if self.actlr & 2 != 0 {
                return Err(Trap::Bus(CFSR_PRECISERR, Some(addr)));
            }
            // buffered write: imprecise BusFault
            self.cfsr |= CFSR_IMPRECISERR;
            if self.shcsr_ena & (1 << 17) != 0 {
                self.set_pending(EXC_BUSFAULT);
            } else {
                self.hfsr |= HFSR_FORCED;
                self.set_pending(EXC_HARDFAULT);
            }
        }
        Ok(())
    }

    /// Debugger memory read (no side effects, no faults).
    pub fn debug_read<B: Bus>(&mut self, bus: &mut B, addr: u32, size: u8) -> Option<u32> {
        if addr >> 20 == 0xE00 {
            return self.ppb_read(addr, size, true).ok();
        }
        bus.read(self.now, addr, size, true).ok()
    }

    /// Debugger memory write.
    pub fn debug_write<B: Bus>(&mut self, bus: &mut B, addr: u32, size: u8, v: u32) -> bool {
        if addr >> 20 == 0xE00 {
            let ok = self.ppb_write(addr, size, v, true).is_ok();
            self.refresh_watch();
            return ok;
        }
        bus.write(self.now, addr, size, v, true).is_ok()
    }

    fn ls_address(&self, rn: u8, addr: Addr) -> (u32, Option<u32>) {
        match addr {
            Addr::Imm { imm, add, index, wback } => {
                let base = self.reg(rn);
                let off = if add { base.wrapping_add(imm) } else { base.wrapping_sub(imm) };
                (if index { off } else { base }, if wback { Some(off) } else { None })
            }
            Addr::Reg { rm, shift } => (self.reg(rn).wrapping_add(self.reg(rm) << shift), None),
            Addr::Literal { imm, add } => {
                let base = self.pc.wrapping_add(4) & !3;
                (if add { base.wrapping_add(imm) } else { base.wrapping_sub(imm) }, None)
            }
        }
    }

    fn dp(&mut self, op: DpOp, s: bool, rd: u8, a: u32, b: u32, shift_carry: bool) -> u32 {
        use DpOp::*;
        let (r, c, v, logical) = match op {
            And | Tst => (a & b, shift_carry, false, true),
            Bic => (a & !b, shift_carry, false, true),
            Orr => (a | b, shift_carry, false, true),
            Orn => (a | !b, shift_carry, false, true),
            Eor | Teq => (a ^ b, shift_carry, false, true),
            Mov => (b, shift_carry, false, true),
            Mvn => (!b, shift_carry, false, true),
            Add | Cmn => {
                let (r, c, v) = add_with_carry(a, b, false);
                (r, c, v, false)
            }
            Adc => {
                let (r, c, v) = add_with_carry(a, b, self.c);
                (r, c, v, false)
            }
            Sub | Cmp => {
                let (r, c, v) = add_with_carry(a, !b, true);
                (r, c, v, false)
            }
            Sbc => {
                let (r, c, v) = add_with_carry(a, !b, self.c);
                (r, c, v, false)
            }
            Rsb => {
                let (r, c, v) = add_with_carry(b, !a, true);
                (r, c, v, false)
            }
        };
        if s {
            self.n = r >> 31 != 0;
            self.z = r == 0;
            self.c = c;
            if !logical {
                self.v = v;
            }
        }
        if matches!(op, Tst | Teq | Cmp | Cmn) {
            return 1;
        }
        if rd == 15 {
            self.next_pc = r & !1;
            return BRANCH_CYCLES;
        }
        self.set_reg(rd, r);
        1
    }

    fn set_nz(&mut self, r: u32) {
        self.n = r >> 31 != 0;
        self.z = r == 0;
    }

    /// Executes a decoded instruction whose condition passed. Returns the cycle count.
    pub(crate) fn execute<B: Bus>(&mut self, bus: &mut B, ins: Instr) -> Result<u32, Trap> {
        use Instr::*;
        match ins {
            Undefined => Err(Trap::Usage(CFSR_UNDEFINSTR)),
            NoCoproc => Err(Trap::Usage(CFSR_NOCP)),
            DpImm { op, s, rd, rn, imm, carry } => {
                let a = if matches!(op, DpOp::Mov | DpOp::Mvn) { 0 } else { self.reg(rn) };
                let c = match carry {
                    ImmCarry::Keep => self.c,
                    ImmCarry::Clear => false,
                    ImmCarry::Set => true,
                };
                let s = self.sflag(s);
                Ok(self.dp(op, s, rd, a, imm, c))
            }
            DpReg { op, s, rd, rn, rm, shift } => {
                let (b, c) = shift_c(self.reg(rm), shift.ty, shift.n as u32, self.c);
                let a = if matches!(op, DpOp::Mov | DpOp::Mvn) { 0 } else { self.reg(rn) };
                let s = self.sflag(s);
                Ok(self.dp(op, s, rd, a, b, c))
            }
            ShiftReg { s, ty, rd, rn, rm } => {
                let (r, c) = shift_c(self.reg(rn), ty, self.reg(rm) & 0xFF, self.c);
                if self.sflag(s) {
                    self.set_nz(r);
                    self.c = c;
                }
                self.set_reg(rd, r);
                Ok(1)
            }
            Adr { rd, imm, add } => {
                let base = self.pc.wrapping_add(4) & !3;
                self.set_reg(rd, if add { base.wrapping_add(imm) } else { base.wrapping_sub(imm) });
                Ok(1)
            }
            MovT { rd, imm } => {
                let v = ((imm as u32) << 16) | (self.reg(rd) & 0xFFFF);
                self.set_reg(rd, v);
                Ok(1)
            }
            Mul { s, rd, rn, rm } => {
                let r = self.reg(rn).wrapping_mul(self.reg(rm));
                if self.sflag(s) {
                    self.set_nz(r);
                }
                self.set_reg(rd, r);
                Ok(1)
            }
            Mla { sub, rd, rn, rm, ra } => {
                let p = self.reg(rn).wrapping_mul(self.reg(rm));
                let a = self.reg(ra);
                self.set_reg(rd, if sub { a.wrapping_sub(p) } else { a.wrapping_add(p) });
                Ok(2)
            }
            LongMul { op, rdlo, rdhi, rn, rm } => {
                let (a, b) = (self.reg(rn), self.reg(rm));
                let acc = ((self.reg(rdhi) as u64) << 32) | self.reg(rdlo) as u64;
                let r: u64 = match op {
                    self::LongMul::Smull => ((a as i32 as i64) * (b as i32 as i64)) as u64,
                    self::LongMul::Umull => (a as u64) * (b as u64),
                    self::LongMul::Smlal => (((a as i32 as i64) * (b as i32 as i64)) as u64).wrapping_add(acc),
                    self::LongMul::Umlal => ((a as u64) * (b as u64)).wrapping_add(acc),
                    self::LongMul::Umaal => (a as u64) * (b as u64) + self.reg(rdhi) as u64 + self.reg(rdlo) as u64,
                };
                self.set_reg(rdlo, r as u32);
                self.set_reg(rdhi, (r >> 32) as u32);
                Ok(1)
            }
            Div { signed, rd, rn, rm } => {
                let (a, b) = (self.reg(rn), self.reg(rm));
                if b == 0 {
                    if self.ccr & (1 << 4) != 0 {
                        return Err(Trap::Usage(CFSR_DIVBYZERO));
                    }
                    self.set_reg(rd, 0);
                    return Ok(2);
                }
                let r = if signed { (a as i32).wrapping_div(b as i32) as u32 } else { a / b };
                self.set_reg(rd, r);
                let bits = (32 - a.leading_zeros()).saturating_sub(32 - b.leading_zeros());
                Ok(2 + (bits / 4).min(10))
            }
            SmlaXy { rd, rn, rm, ra, nh, mh } => {
                let h = |v: u32, hi: bool| if hi { (v >> 16) as i16 as i32 } else { v as i16 as i32 };
                let p = h(self.reg(rn), nh) * h(self.reg(rm), mh);
                let r = if ra == 15 {
                    p
                } else {
                    let full = p as i64 + self.reg(ra) as i32 as i64;
                    if full != full as i32 as i64 {
                        self.q = true;
                    }
                    full as i32
                };
                self.set_reg(rd, r as u32);
                Ok(1)
            }
            SmlaWy { rd, rn, rm, ra, mh } => {
                let m = if mh { (self.reg(rm) >> 16) as i16 as i64 } else { self.reg(rm) as i16 as i64 };
                let p = ((self.reg(rn) as i32 as i64) * m) >> 16;
                let r = if ra == 15 {
                    p as i32
                } else {
                    let full = p + self.reg(ra) as i32 as i64;
                    if full != full as i32 as i64 {
                        self.q = true;
                    }
                    full as i32
                };
                self.set_reg(rd, r as u32);
                Ok(1)
            }
            Smlad { sub, x, rd, rn, rm, ra } => {
                let a = self.reg(rn);
                let b = if x { self.reg(rm).rotate_right(16) } else { self.reg(rm) };
                let p1 = (a as i16 as i64) * (b as i16 as i64);
                let p2 = ((a >> 16) as i16 as i64) * ((b >> 16) as i16 as i64);
                let mut sum = if sub { p1 - p2 } else { p1 + p2 };
                if ra != 15 {
                    sum += self.reg(ra) as i32 as i64;
                }
                if sum != sum as i32 as i64 {
                    self.q = true;
                }
                self.set_reg(rd, sum as i32 as u32);
                Ok(1)
            }
            Smlald { sub, x, rdlo, rdhi, rn, rm } => {
                let a = self.reg(rn);
                let b = if x { self.reg(rm).rotate_right(16) } else { self.reg(rm) };
                let p1 = (a as i16 as i64) * (b as i16 as i64);
                let p2 = ((a >> 16) as i16 as i64) * ((b >> 16) as i16 as i64);
                let acc = (((self.reg(rdhi) as u64) << 32) | self.reg(rdlo) as u64) as i64;
                let r = acc.wrapping_add(if sub { p1 - p2 } else { p1 + p2 }) as u64;
                self.set_reg(rdlo, r as u32);
                self.set_reg(rdhi, (r >> 32) as u32);
                Ok(1)
            }
            SmlalXy { rdlo, rdhi, rn, rm, nh, mh } => {
                let h = |v: u32, hi: bool| if hi { (v >> 16) as i16 as i64 } else { v as i16 as i64 };
                let p = h(self.reg(rn), nh) * h(self.reg(rm), mh);
                let acc = (((self.reg(rdhi) as u64) << 32) | self.reg(rdlo) as u64) as i64;
                let r = acc.wrapping_add(p) as u64;
                self.set_reg(rdlo, r as u32);
                self.set_reg(rdhi, (r >> 32) as u32);
                Ok(1)
            }
            Smmla { sub, round, rd, rn, rm, ra } => {
                let p = (self.reg(rn) as i32 as i64) * (self.reg(rm) as i32 as i64);
                let acc = if ra == 15 { 0 } else { (self.reg(ra) as i32 as i64) << 32 };
                let mut r = if sub { acc.wrapping_sub(p) } else { acc.wrapping_add(p) };
                if round {
                    r = r.wrapping_add(0x8000_0000);
                }
                self.set_reg(rd, (r >> 32) as u32);
                Ok(1)
            }
            Usada8 { rd, rn, rm, ra } => {
                let (a, b) = (self.reg(rn), self.reg(rm));
                let mut sum: u32 = (0..4).map(|i| (((a >> (8 * i)) & 0xFF) as i32 - ((b >> (8 * i)) & 0xFF) as i32).unsigned_abs()).sum();
                if ra != 15 {
                    sum = sum.wrapping_add(self.reg(ra));
                }
                self.set_reg(rd, sum);
                Ok(1)
            }
            Parallel { op, kind, rd, rn, rm } => {
                let r = self.parallel(op, kind, self.reg(rn), self.reg(rm));
                self.set_reg(rd, r);
                Ok(1)
            }
            Sat { signed, rd, rn, sat, shift } => {
                let operand = crate::alu::shift(self.reg(rn), shift.ty, shift.n as u32, self.c) as i32 as i64;
                let (r, sat_hit) = if signed {
                    let (r, s) = signed_sat_q(operand, sat as u32);
                    (r as u32, s)
                } else {
                    unsigned_sat_q(operand, sat as u32)
                };
                if sat_hit {
                    self.q = true;
                }
                self.set_reg(rd, r);
                Ok(1)
            }
            Sat16 { signed, rd, rn, sat } => {
                let v = self.reg(rn);
                let mut r = 0u32;
                for i in 0..2 {
                    let h = ((v >> (16 * i)) as i16) as i64;
                    let (x, s) = if signed {
                        let (x, s) = signed_sat_q(h, sat as u32);
                        (x as u32, s)
                    } else {
                        unsigned_sat_q(h, sat as u32)
                    };
                    if s {
                        self.q = true;
                    }
                    r |= (x & 0xFFFF) << (16 * i);
                }
                self.set_reg(rd, r);
                Ok(1)
            }
            Qop { op, rd, rn, rm } => {
                let m = self.reg(rm) as i32 as i64;
                let n = self.reg(rn) as i32 as i64;
                let mut sat = false;
                let mut s32 = |x: i64| {
                    let (r, s) = signed_sat_q(x, 32);
                    sat |= s;
                    r as i64
                };
                let r = match op {
                    QOp::Qadd => s32(m + n),
                    QOp::Qsub => s32(m - n),
                    QOp::Qdadd => {
                        let d = s32(2 * n);
                        s32(m + d)
                    }
                    QOp::Qdsub => {
                        let d = s32(2 * n);
                        s32(m - d)
                    }
                };
                if sat {
                    self.q = true;
                }
                self.set_reg(rd, r as i32 as u32);
                Ok(1)
            }
            Sel { rd, rn, rm } => {
                let (a, b) = (self.reg(rn), self.reg(rm));
                let mut r = 0;
                for i in 0..4 {
                    let src = if self.ge & (1 << i) != 0 { a } else { b };
                    r |= src & (0xFF << (8 * i));
                }
                self.set_reg(rd, r);
                Ok(1)
            }
            Clz { rd, rm } => {
                let v = self.reg(rm).leading_zeros();
                self.set_reg(rd, v);
                Ok(1)
            }
            Rbit { rd, rm } => {
                let v = self.reg(rm).reverse_bits();
                self.set_reg(rd, v);
                Ok(1)
            }
            Rev { rd, rm } => {
                let v = self.reg(rm).swap_bytes();
                self.set_reg(rd, v);
                Ok(1)
            }
            Rev16 { rd, rm } => {
                let v = self.reg(rm);
                self.set_reg(rd, ((v & 0x00FF_00FF) << 8) | ((v & 0xFF00_FF00) >> 8));
                Ok(1)
            }
            Revsh { rd, rm } => {
                let v = self.reg(rm);
                self.set_reg(rd, ((v as u16).swap_bytes() as i16) as i32 as u32);
                Ok(1)
            }
            Extend { signed, bits, rd, rn, rm, rot } => {
                let v = self.reg(rm).rotate_right(rot as u32);
                let r = match bits {
                    8 => {
                        let x = if signed { v as i8 as i32 as u32 } else { v & 0xFF };
                        if rn == 15 {
                            x
                        } else {
                            self.reg(rn).wrapping_add(x)
                        }
                    }
                    16 => {
                        let x = if signed { v as i16 as i32 as u32 } else { v & 0xFFFF };
                        if rn == 15 {
                            x
                        } else {
                            self.reg(rn).wrapping_add(x)
                        }
                    }
                    _ => {
                        let lane = |b: u32| if signed { (b as i8 as i16) as u16 as u32 } else { b & 0xFF };
                        let (lo, hi) = (lane(v), lane(v >> 16));
                        if rn == 15 {
                            lo | (hi << 16)
                        } else {
                            let a = self.reg(rn);
                            ((a as u16).wrapping_add(lo as u16) as u32) | (((a >> 16) as u16).wrapping_add(hi as u16) as u32) << 16
                        }
                    }
                };
                self.set_reg(rd, r);
                Ok(1)
            }
            Bfi { rd, rn, lsb, msb } => {
                if msb < lsb {
                    return Ok(1);
                }
                let width = (msb - lsb + 1) as u32;
                let mask = (if width == 32 { u32::MAX } else { (1u32 << width) - 1 }) << lsb;
                let src = if rn == 15 { 0 } else { self.reg(rn) };
                let v = (self.reg(rd) & !mask) | ((src << lsb) & mask);
                self.set_reg(rd, v);
                Ok(1)
            }
            Bfx { signed, rd, rn, lsb, widthm1 } => {
                let width = widthm1 as u32 + 1;
                if lsb as u32 + width > 32 {
                    return Ok(1);
                }
                let v = self.reg(rn) >> lsb;
                let r = if width == 32 {
                    v
                } else if signed {
                    let s = 32 - width;
                    (((v << s) as i32) >> s) as u32
                } else {
                    v & ((1 << width) - 1)
                };
                self.set_reg(rd, r);
                Ok(1)
            }
            Pkh { tb, rd, rn, rm, shift } => {
                let op2 = crate::alu::shift(self.reg(rm), shift.ty, shift.n as u32, self.c);
                let n = self.reg(rn);
                let r = if tb { (n & 0xFFFF_0000) | (op2 & 0xFFFF) } else { (n & 0xFFFF) | (op2 & 0xFFFF_0000) };
                self.set_reg(rd, r);
                Ok(1)
            }
            B { cond, imm } => {
                if cond != 14 && !condition_passed(cond, self.n, self.z, self.c, self.v) {
                    return Ok(1);
                }
                self.next_pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                Ok(BRANCH_CYCLES)
            }
            Bl { imm } => {
                self.r[14] = self.next_pc | 1;
                self.next_pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                Ok(BRANCH_CYCLES + 1)
            }
            Bx { rm } => {
                let t = self.reg(rm);
                self.bx_write_pc(t);
                Ok(BRANCH_CYCLES)
            }
            Blx { rm } => {
                let target = self.reg(rm);
                self.r[14] = self.pc.wrapping_add(2) | 1;
                self.t = target & 1 != 0;
                self.next_pc = target & !1;
                Ok(BRANCH_CYCLES)
            }
            Cbz { nonzero, rn, imm } => {
                if (self.reg(rn) != 0) == nonzero {
                    self.next_pc = self.pc.wrapping_add(4).wrapping_add(imm);
                    Ok(BRANCH_CYCLES)
                } else {
                    Ok(1)
                }
            }
            Tb { half, rn, rm } => {
                let base = self.reg(rn);
                let idx = self.reg(rm);
                let off = if half {
                    self.mem_read(bus, base.wrapping_add(idx << 1), 2, false, false)?
                } else {
                    self.mem_read(bus, base.wrapping_add(idx), 1, false, false)?
                };
                self.next_pc = self.pc.wrapping_add(4).wrapping_add(2 * off);
                Ok(2 + BRANCH_CYCLES)
            }
            Ldr { size, signed, unpriv, rt, rn, addr } => {
                let (a, wb) = self.ls_address(rn, addr);
                let mut v = self.mem_read(bus, a, size, false, unpriv)?;
                if signed {
                    v = match size {
                        1 => v as i8 as i32 as u32,
                        2 => v as i16 as i32 as u32,
                        _ => v,
                    };
                }
                if let Some(w) = wb {
                    self.set_reg(rn, w);
                }
                if rt == 15 {
                    if a & 3 != 0 {
                        return Err(Trap::Usage(CFSR_UNALIGNED));
                    }
                    self.bx_write_pc(v);
                    return Ok(2 + BRANCH_CYCLES);
                }
                self.set_reg(rt, v);
                Ok(2)
            }
            Str { size, unpriv, rt, rn, addr } => {
                let (a, wb) = self.ls_address(rn, addr);
                let v = self.reg(rt);
                self.mem_write(bus, a, size, v, false, unpriv)?;
                if let Some(w) = wb {
                    self.set_reg(rn, w);
                }
                Ok(2)
            }
            Ldrd { rt, rt2, rn, imm, add, index, wback } => {
                let base = if rn == 15 { self.pc.wrapping_add(4) & !3 } else { self.reg(rn) };
                let off = if add { base.wrapping_add(imm) } else { base.wrapping_sub(imm) };
                let a = if index { off } else { base };
                let v1 = self.mem_read(bus, a, 4, true, false)?;
                let v2 = self.mem_read(bus, a.wrapping_add(4), 4, true, false)?;
                if wback {
                    self.set_reg(rn, off);
                }
                self.set_reg(rt, v1);
                self.set_reg(rt2, v2);
                Ok(3)
            }
            Strd { rt, rt2, rn, imm, add, index, wback } => {
                let base = self.reg(rn);
                let off = if add { base.wrapping_add(imm) } else { base.wrapping_sub(imm) };
                let a = if index { off } else { base };
                let (v1, v2) = (self.reg(rt), self.reg(rt2));
                self.mem_write(bus, a, 4, v1, true, false)?;
                self.mem_write(bus, a.wrapping_add(4), 4, v2, true, false)?;
                if wback {
                    self.set_reg(rn, off);
                }
                Ok(3)
            }
            Ldm { rn, regs, wback, db } => {
                let base = self.reg(rn);
                let count = regs.count_ones();
                let start = if db { base.wrapping_sub(4 * count) } else { base };
                let mut vals = [0u32; 16];
                let mut a = start;
                for (i, val) in vals.iter_mut().enumerate() {
                    if regs & (1 << i) != 0 {
                        *val = self.mem_read(bus, a, 4, true, false)?;
                        a = a.wrapping_add(4);
                    }
                }
                if wback {
                    let nb = if db { base.wrapping_sub(4 * count) } else { base.wrapping_add(4 * count) };
                    self.set_reg(rn, nb);
                }
                for (i, v) in vals.iter().enumerate().take(15) {
                    if regs & (1 << i) != 0 {
                        self.set_reg(i as u8, *v);
                    }
                }
                if regs & 0x8000 != 0 {
                    self.bx_write_pc(vals[15]);
                    return Ok(1 + count + BRANCH_CYCLES - 1);
                }
                Ok(1 + count)
            }
            Stm { rn, regs, wback, db } => {
                let base = self.reg(rn);
                let count = regs.count_ones();
                let start = if db { base.wrapping_sub(4 * count) } else { base };
                let mut a = start;
                for i in 0..16u8 {
                    if regs & (1 << i) != 0 {
                        let v = self.reg(i);
                        self.mem_write(bus, a, 4, v, true, false)?;
                        a = a.wrapping_add(4);
                    }
                }
                if wback {
                    let nb = if db { base.wrapping_sub(4 * count) } else { base.wrapping_add(4 * count) };
                    self.set_reg(rn, nb);
                }
                Ok(1 + count)
            }
            Ldrex { size, rt, rn, imm } => {
                let a = self.reg(rn).wrapping_add(imm);
                let v = self.mem_read(bus, a, size, true, false)?;
                self.excl = true;
                self.set_reg(rt, v);
                Ok(2)
            }
            Strex { size, rd, rt, rn, imm } => {
                let a = self.reg(rn).wrapping_add(imm);
                if size > 1 && a & (size as u32 - 1) != 0 {
                    return Err(Trap::Usage(CFSR_UNALIGNED));
                }
                if self.excl {
                    let v = self.reg(rt);
                    self.mem_write(bus, a, size, v, true, false)?;
                    self.set_reg(rd, 0);
                } else {
                    self.set_reg(rd, 1);
                }
                self.excl = false;
                Ok(2)
            }
            Clrex => {
                self.excl = false;
                Ok(1)
            }
            MemHint => Ok(1),
            Mrs { rd, sysm } => {
                let v = match sysm {
                    0..=7 => {
                        let mut v = 0;
                        if sysm & 1 != 0 {
                            v |= self.ipsr & 0x1FF;
                        }
                        if sysm & 4 == 0 {
                            v |= self.apsr();
                        }
                        v
                    }
                    8 if self.privileged() => self.msp(),
                    9 if self.privileged() => self.psp(),
                    16 => self.primask as u32,
                    17 | 18 => self.basepri as u32,
                    19 => self.faultmask as u32,
                    20 => self.control(),
                    _ => 0,
                };
                self.set_reg(rd, v);
                Ok(2)
            }
            Msr { rn, sysm, mask } => {
                let v = self.reg(rn);
                match sysm {
                    0..=3 => {
                        if mask & 2 != 0 {
                            self.set_apsr_nzcvq(v);
                        }
                        if mask & 1 != 0 {
                            self.ge = ((v >> 16) & 0xF) as u8;
                        }
                    }
                    _ if !self.privileged() => {}
                    8 => self.set_msp(v),
                    9 => self.set_psp(v),
                    16 => self.set_primask(v & 1 != 0),
                    17 => self.set_basepri(v as u8),
                    18 => {
                        let nv = (v as u8) & 0xE0;
                        if nv != 0 && (nv < self.basepri || self.basepri == 0) {
                            self.set_basepri(nv);
                        }
                    }
                    19 if self.execution_priority() > -1 => {
                        self.set_faultmask(v & 1 != 0);
                    }
                    20 => {
                        self.npriv = v & 1 != 0;
                        if !self.handler_mode() {
                            let ipsr = self.ipsr;
                            self.set_stack_state(ipsr, v & 2 != 0);
                        }
                        self.fpca = v & 4 != 0;
                    }
                    _ => {}
                }
                Ok(2)
            }
            Cps { disable, i, f } => {
                if self.privileged() {
                    if disable {
                        if i {
                            self.set_primask(true);
                        }
                        if f && self.execution_priority() > -1 {
                            self.set_faultmask(true);
                        }
                    } else {
                        if i {
                            self.set_primask(false);
                        }
                        if f {
                            self.set_faultmask(false);
                        }
                    }
                }
                Ok(1)
            }
            Barrier(_) => Ok(1),
            Hint(h) => {
                match h {
                    self::Hint::Nop | self::Hint::Yield => {}
                    self::Hint::Wfi => {
                        self.sleeping = true;
                        self.sleep_wfe = false;
                    }
                    self::Hint::Wfe => {
                        if self.event_reg {
                            self.event_reg = false;
                        } else {
                            self.sleeping = true;
                            self.sleep_wfe = true;
                        }
                    }
                    self::Hint::Sev => self.event_reg = true,
                }
                Ok(1)
            }
            It { firstcond, mask } => {
                self.it = (firstcond << 4) | mask;
                Ok(1)
            }
            Bkpt { imm } => Err(Trap::Bkpt(imm)),
            Svc { .. } => Err(Trap::Svc),
            Vfp(v) => self.exec_vfp(bus, v),
        }
    }

    fn parallel(&mut self, op: ParOp, kind: ParKind, a: u32, b: u32) -> u32 {
        let signed = matches!(kind, ParKind::S | ParKind::Q | ParKind::Sh);
        let mut r = 0u32;
        let mut ge = 0u8;
        let lanes16 = matches!(op, ParOp::Add16 | ParOp::Sub16 | ParOp::Asx | ParOp::Sax);
        if lanes16 {
            let get = |v: u32, i: u32| -> i64 {
                let h = (v >> (16 * i)) & 0xFFFF;
                if signed {
                    h as u16 as i16 as i64
                } else {
                    h as i64
                }
            };
            let (a0, a1, b0, b1) = (get(a, 0), get(a, 1), get(b, 0), get(b, 1));
            let (r0, add0, r1, add1) = match op {
                ParOp::Add16 => (a0 + b0, true, a1 + b1, true),
                ParOp::Sub16 => (a0 - b0, false, a1 - b1, false),
                ParOp::Asx => (a0 - b1, false, a1 + b0, true),
                _ => (a0 + b1, true, a1 - b0, false),
            };
            for (i, (x, add)) in [(r0, add0), (r1, add1)].into_iter().enumerate() {
                let v: u32 = match kind {
                    ParKind::S => {
                        if x >= 0 {
                            ge |= 0b11 << (2 * i);
                        }
                        x as u32 & 0xFFFF
                    }
                    ParKind::U => {
                        if (add && x >= 0x1_0000) || (!add && x >= 0) {
                            ge |= 0b11 << (2 * i);
                        }
                        x as u32 & 0xFFFF
                    }
                    ParKind::Q => signed_sat_q(x, 16).0 as u32 & 0xFFFF,
                    ParKind::Uq => unsigned_sat_q(x, 16).0,
                    ParKind::Sh | ParKind::Uh => (x >> 1) as u32 & 0xFFFF,
                };
                r |= v << (16 * i);
            }
        } else {
            for i in 0..4 {
                let get = |v: u32| -> i64 {
                    let h = (v >> (8 * i)) & 0xFF;
                    if signed {
                        h as u8 as i8 as i64
                    } else {
                        h as i64
                    }
                };
                let add = op == ParOp::Add8;
                let x = if add { get(a) + get(b) } else { get(a) - get(b) };
                let v: u32 = match kind {
                    ParKind::S => {
                        if x >= 0 {
                            ge |= 1 << i;
                        }
                        x as u32 & 0xFF
                    }
                    ParKind::U => {
                        if (add && x >= 0x100) || (!add && x >= 0) {
                            ge |= 1 << i;
                        }
                        x as u32 & 0xFF
                    }
                    ParKind::Q => signed_sat_q(x, 8).0 as u32 & 0xFF,
                    ParKind::Uq => unsigned_sat_q(x, 8).0,
                    ParKind::Sh | ParKind::Uh => (x >> 1) as u32 & 0xFF,
                };
                r |= v << (8 * i);
            }
        }
        if matches!(kind, ParKind::S | ParKind::U) {
            self.ge = ge;
        }
        r
    }

    /// `ExecuteFPCheck()`: access check, lazy state preservation, FP context creation.
    fn fp_check<B: Bus>(&mut self, bus: &mut B) -> Result<(), Trap> {
        let cp10 = (self.cpacr >> 20) & 3;
        if cp10 == 0 || cp10 == 2 || (cp10 == 1 && !self.privileged()) {
            return Err(Trap::Usage(CFSR_NOCP));
        }
        if self.fpccr & 1 != 0 {
            self.preserve_fp_state(bus);
        }
        if self.fpccr & (1 << 31) != 0 && !self.fpca {
            self.fpscr = (self.fpscr & !0x07C0_0000) | (self.fpdscr & 0x07C0_0000);
            self.fpca = true;
        }
        Ok(())
    }

    /// `PreserveFPState()` of lazy stacking.
    fn preserve_fp_state<B: Bus>(&mut self, bus: &mut B) {
        let base = self.fpcar;
        let mut err = false;
        for i in 0..16u32 {
            if bus.write(self.now, base + 4 * i, 4, self.s[i as usize], false).is_err() {
                err = true;
            }
        }
        if bus.write(self.now, base + 0x40, 4, self.fpscr, false).is_err() {
            err = true;
        }
        if err {
            self.cfsr |= CFSR_LSPERR;
            if self.fpccr & (1 << 6) != 0 {
                self.set_pending(EXC_BUSFAULT);
            } else {
                self.hfsr |= HFSR_FORCED;
                self.set_pending(EXC_HARDFAULT);
            }
        }
        self.fpccr &= !1;
    }

    fn exec_vfp<B: Bus>(&mut self, bus: &mut B, v: Vfp) -> Result<u32, Trap> {
        self.fp_check(bus)?;
        let mut st = self.fpscr;
        let cycles = match v {
            Vfp::Arith { op, sd, sn, sm } => {
                let (d, n, m) = (self.s[sd as usize], self.s[sn as usize], self.s[sm as usize]);
                let (r, c) = match op {
                    VArith::Mla => {
                        let p = fpu::mul(n, m, &mut st);
                        (fpu::add(d, p, &mut st), 3)
                    }
                    VArith::Mls => {
                        let p = fpu::mul(n, m, &mut st);
                        (fpu::add(d, fpu::neg(p), &mut st), 3)
                    }
                    VArith::Nmla => {
                        let p = fpu::mul(n, m, &mut st);
                        (fpu::add(fpu::neg(d), fpu::neg(p), &mut st), 3)
                    }
                    VArith::Nmls => {
                        let p = fpu::mul(n, m, &mut st);
                        (fpu::add(fpu::neg(d), p, &mut st), 3)
                    }
                    VArith::Mul => (fpu::mul(n, m, &mut st), 1),
                    VArith::Nmul => (fpu::neg(fpu::mul(n, m, &mut st)), 1),
                    VArith::Add => (fpu::add(n, m, &mut st), 1),
                    VArith::Sub => (fpu::sub(n, m, &mut st), 1),
                    VArith::Div => (fpu::div(n, m, &mut st), 14),
                    VArith::Fma => (fpu::mul_add(d, n, m, &mut st), 3),
                    VArith::Fms => (fpu::mul_add(d, fpu::neg(n), m, &mut st), 3),
                    VArith::Fnma => (fpu::mul_add(fpu::neg(d), fpu::neg(n), m, &mut st), 3),
                    VArith::Fnms => (fpu::mul_add(fpu::neg(d), n, m, &mut st), 3),
                };
                self.s[sd as usize] = r;
                c
            }
            Vfp::Unary { op, sd, sm } => {
                let m = self.s[sm as usize];
                let (r, c) = match op {
                    VUnary::Mov => (m, 1),
                    VUnary::Abs => (fpu::abs(m), 1),
                    VUnary::Neg => (fpu::neg(m), 1),
                    VUnary::Sqrt => (fpu::sqrt(m, &mut st), 14),
                };
                self.s[sd as usize] = r;
                c
            }
            Vfp::MovImm { sd, imm } => {
                self.s[sd as usize] = imm;
                1
            }
            Vfp::Cmp { e, zero, sd, sm } => {
                let m = if zero { 0 } else { self.s[sm as usize] };
                let flags = fpu::compare(self.s[sd as usize], m, e, &mut st);
                st = (st & 0x0FFF_FFFF) | flags;
                1
            }
            Vfp::CvtToInt { signed, round_zero, sd, sm } => {
                self.s[sd as usize] = fpu::to_fixed(self.s[sm as usize], 32, 0, !signed, round_zero, &mut st);
                1
            }
            Vfp::CvtFromInt { signed, sd, sm } => {
                self.s[sd as usize] = fpu::from_fixed(self.s[sm as usize], 32, 0, !signed, false, &mut st);
                1
            }
            Vfp::CvtFixed { to_fixed, signed, bits, frac, sd } => {
                let x = self.s[sd as usize];
                self.s[sd as usize] = if to_fixed {
                    fpu::to_fixed(x, bits as u32, frac as u32, !signed, true, &mut st)
                } else {
                    fpu::from_fixed(x, bits as u32, frac as u32, !signed, true, &mut st)
                };
                1
            }
            Vfp::CvtHalf { to_half, top, sd, sm } => {
                if to_half {
                    let h = fpu::single_to_half(self.s[sm as usize], &mut st) as u32;
                    let d = self.s[sd as usize];
                    self.s[sd as usize] = if top { (d & 0xFFFF) | (h << 16) } else { (d & 0xFFFF_0000) | h };
                } else {
                    let m = self.s[sm as usize];
                    let h = if top { m >> 16 } else { m & 0xFFFF } as u16;
                    self.s[sd as usize] = fpu::half_to_single(h, &mut st);
                }
                1
            }
            Vfp::LdrStr { load, double, sd, rn, imm, add } => {
                let base = if rn == 15 { self.pc.wrapping_add(4) & !3 } else { self.reg(rn) };
                let a = if add { base.wrapping_add(imm) } else { base.wrapping_sub(imm) };
                let words = if double { 2 } else { 1 };
                for i in 0..words {
                    let idx = (sd as usize + i) & 31;
                    let addr = a.wrapping_add(4 * i as u32);
                    if load {
                        self.s[idx] = self.mem_read(bus, addr, 4, true, false)?;
                    } else {
                        let v = self.s[idx];
                        self.mem_write(bus, addr, 4, v, true, false)?;
                    }
                }
                2
            }
            Vfp::LdmStm { load, sd, count, rn, inc, wback } => {
                let base = self.reg(rn);
                let bytes = 4 * count as u32;
                let start = if inc { base } else { base.wrapping_sub(bytes) };
                for i in 0..count as usize {
                    let idx = (sd as usize + i) & 31;
                    let addr = start.wrapping_add(4 * i as u32);
                    if load {
                        self.s[idx] = self.mem_read(bus, addr, 4, true, false)?;
                    } else {
                        let v = self.s[idx];
                        self.mem_write(bus, addr, 4, v, true, false)?;
                    }
                }
                if wback {
                    self.set_reg(rn, if inc { base.wrapping_add(bytes) } else { base.wrapping_sub(bytes) });
                }
                1 + count as u32
            }
            Vfp::MovCore { to_core, rt, sn } => {
                if to_core {
                    let v = self.s[sn as usize];
                    self.set_reg(rt, v);
                } else {
                    self.s[sn as usize] = self.reg(rt);
                }
                1
            }
            Vfp::MovCore2 { to_core, rt, rt2, sm } => {
                let (i, j) = (sm as usize & 31, (sm as usize + 1) & 31);
                if to_core {
                    let (a, b) = (self.s[i], self.s[j]);
                    self.set_reg(rt, a);
                    self.set_reg(rt2, b);
                } else {
                    self.s[i] = self.reg(rt);
                    self.s[j] = self.reg(rt2);
                }
                2
            }
            Vfp::MovScalar { to_core, rt, sm } => {
                if to_core {
                    let v = self.s[sm as usize & 31];
                    self.set_reg(rt, v);
                } else {
                    self.s[sm as usize & 31] = self.reg(rt);
                }
                1
            }
            Vfp::Vmrs { rt } => {
                if rt == 15 {
                    self.n = st >> 31 & 1 != 0;
                    self.z = st >> 30 & 1 != 0;
                    self.c = st >> 29 & 1 != 0;
                    self.v = st >> 28 & 1 != 0;
                } else {
                    self.set_reg(rt, st);
                }
                1
            }
            Vfp::Vmsr { rt } => {
                st = self.reg(rt) & fpu::FPSCR_MASK;
                1
            }
        };
        self.fpscr = st;
        Ok(cycles)
    }
}
