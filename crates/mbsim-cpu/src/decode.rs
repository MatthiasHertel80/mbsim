//! Thumb and Thumb-2 decoder for ARMv7E-M with the FPv4-SP extension.
//!
//! Section references are to the ARMv7-M Architecture Reference Manual (DDI 0403E):
//! A5.2 (16-bit encodings), A5.3 (32-bit encodings), A6 (floating-point encodings).

/// Data-processing operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DpOp {
    And,
    Bic,
    Orr,
    Orn,
    Eor,
    Add,
    Adc,
    Sbc,
    Sub,
    Rsb,
    Mov,
    Mvn,
    Tst,
    Teq,
    Cmp,
    Cmn,
}

/// When an instruction updates the condition flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetFlags {
    No,
    Yes,
    /// 16-bit encodings: flags are set outside an IT block only.
    OutsideIt,
}

/// Shift types (`DecodeImmShift`).
pub const LSL: u8 = 0;
pub const LSR: u8 = 1;
pub const ASR: u8 = 2;
pub const ROR: u8 = 3;
pub const RRX: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shift {
    pub ty: u8,
    pub n: u8,
}

impl Shift {
    pub const NONE: Shift = Shift { ty: LSL, n: 0 };

    /// `DecodeImmShift(type, imm5)`.
    pub fn imm(ty: u32, imm5: u32) -> Shift {
        match ty & 3 {
            0 => Shift { ty: LSL, n: imm5 as u8 },
            1 => Shift { ty: LSR, n: if imm5 == 0 { 32 } else { imm5 as u8 } },
            2 => Shift { ty: ASR, n: if imm5 == 0 { 32 } else { imm5 as u8 } },
            _ => {
                if imm5 == 0 {
                    Shift { ty: RRX, n: 1 }
                } else {
                    Shift { ty: ROR, n: imm5 as u8 }
                }
            }
        }
    }
}

/// Carry output of a modified immediate (`ThumbExpandImm_C`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImmCarry {
    Keep,
    Clear,
    Set,
}

/// Addressing mode of a single load or store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addr {
    /// `[Rn, #+/-imm]`, pre/post-indexed with optional write-back.
    Imm { imm: u32, add: bool, index: bool, wback: bool },
    /// `[Rn, Rm, LSL #n]`.
    Reg { rm: u8, shift: u8 },
    /// `[PC, #+/-imm]` relative to `Align(PC, 4)`.
    Literal { imm: u32, add: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LongMul {
    Smull,
    Umull,
    Smlal,
    Umlal,
    Umaal,
}

/// Parallel add/subtract operation (A5.3.13, A5.3.14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParOp {
    Add8,
    Add16,
    Asx,
    Sub8,
    Sub16,
    Sax,
}

/// Parallel add/subtract variant: signed, saturating, halving; unsigned, saturating, halving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParKind {
    S,
    Q,
    Sh,
    U,
    Uq,
    Uh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QOp {
    Qadd,
    Qdadd,
    Qsub,
    Qdsub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    Nop,
    Yield,
    Wfe,
    Wfi,
    Sev,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Barrier {
    Dsb,
    Dmb,
    Isb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VArith {
    Mla,
    Mls,
    Nmla,
    Nmls,
    Mul,
    Nmul,
    Add,
    Sub,
    Div,
    Fma,
    Fms,
    Fnma,
    Fnms,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VUnary {
    Mov,
    Abs,
    Neg,
    Sqrt,
}

/// Floating-point instructions (single precision; FPv4-SP-D16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vfp {
    Arith {
        op: VArith,
        sd: u8,
        sn: u8,
        sm: u8,
    },
    Unary {
        op: VUnary,
        sd: u8,
        sm: u8,
    },
    MovImm {
        sd: u8,
        imm: u32,
    },
    Cmp {
        e: bool,
        zero: bool,
        sd: u8,
        sm: u8,
    },
    /// Float to integer; `round_zero` = VCVT (towards zero), otherwise VCVTR (FPSCR mode).
    CvtToInt {
        signed: bool,
        round_zero: bool,
        sd: u8,
        sm: u8,
    },
    CvtFromInt {
        signed: bool,
        sd: u8,
        sm: u8,
    },
    /// Float <-> fixed point in place (`sd`), `bits` = 16 or 32.
    CvtFixed {
        to_fixed: bool,
        signed: bool,
        bits: u8,
        frac: u8,
        sd: u8,
    },
    /// Half <-> single precision; `top` selects the upper half of the half-precision register.
    CvtHalf {
        to_half: bool,
        top: bool,
        sd: u8,
        sm: u8,
    },
    /// VLDR/VSTR; `sd` is an S register index (a D register `n` is S `2n`), `double` moves 64 bits.
    LdrStr {
        load: bool,
        double: bool,
        sd: u8,
        rn: u8,
        imm: u32,
        add: bool,
    },
    /// VLDM/VSTM/VPUSH/VPOP; `count` in S registers.
    LdmStm {
        load: bool,
        sd: u8,
        count: u8,
        rn: u8,
        inc: bool,
        wback: bool,
    },
    /// VMOV Rt <-> Sn.
    MovCore {
        to_core: bool,
        rt: u8,
        sn: u8,
    },
    /// VMOV Rt, Rt2 <-> Sm, Sm+1 (or Dm).
    MovCore2 {
        to_core: bool,
        rt: u8,
        rt2: u8,
        sm: u8,
    },
    /// VMOV Rt <-> Dd[x]; `sm` is the S register of the scalar.
    MovScalar {
        to_core: bool,
        rt: u8,
        sm: u8,
    },
    Vmrs {
        rt: u8,
    },
    Vmsr {
        rt: u8,
    },
}

/// A decoded instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instr {
    /// Undefined (UsageFault UNDEFINSTR).
    Undefined,
    /// Coprocessor instruction for a coprocessor other than CP10/CP11 (UsageFault NOCP).
    NoCoproc,
    DpImm {
        op: DpOp,
        s: SetFlags,
        rd: u8,
        rn: u8,
        imm: u32,
        carry: ImmCarry,
    },
    DpReg {
        op: DpOp,
        s: SetFlags,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: Shift,
    },
    /// LSL/LSR/ASR/ROR (register): Rd = Rn shifted by Rm<7:0>.
    ShiftReg {
        s: SetFlags,
        ty: u8,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    /// ADR: Rd = Align(PC, 4) +/- imm.
    Adr {
        rd: u8,
        imm: u32,
        add: bool,
    },
    MovT {
        rd: u8,
        imm: u16,
    },
    Mul {
        s: SetFlags,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    Mla {
        sub: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    LongMul {
        op: LongMul,
        rdlo: u8,
        rdhi: u8,
        rn: u8,
        rm: u8,
    },
    Div {
        signed: bool,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    /// SMLA<x><y> / SMUL<x><y> (ra = 15).
    SmlaXy {
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
        nh: bool,
        mh: bool,
    },
    /// SMLAW<y> / SMULW<y> (ra = 15).
    SmlaWy {
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
        mh: bool,
    },
    /// SMLAD/SMLSD/SMUAD/SMUSD (ra = 15 for the non-accumulating forms).
    Smlad {
        sub: bool,
        x: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    Smlald {
        sub: bool,
        x: bool,
        rdlo: u8,
        rdhi: u8,
        rn: u8,
        rm: u8,
    },
    SmlalXy {
        rdlo: u8,
        rdhi: u8,
        rn: u8,
        rm: u8,
        nh: bool,
        mh: bool,
    },
    /// SMMLA/SMMLS/SMMUL (ra = 15).
    Smmla {
        sub: bool,
        round: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    /// USADA8 / USAD8 (ra = 15).
    Usada8 {
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    Parallel {
        op: ParOp,
        kind: ParKind,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    /// SSAT/USAT with a shifted operand; `sat` is the saturation bit position.
    Sat {
        signed: bool,
        rd: u8,
        rn: u8,
        sat: u8,
        shift: Shift,
    },
    Sat16 {
        signed: bool,
        rd: u8,
        rn: u8,
        sat: u8,
    },
    /// QADD etc.: Rd = sat(Rm op Rn).
    Qop {
        op: QOp,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    Sel {
        rd: u8,
        rn: u8,
        rm: u8,
    },
    Clz {
        rd: u8,
        rm: u8,
    },
    Rbit {
        rd: u8,
        rm: u8,
    },
    Rev {
        rd: u8,
        rm: u8,
    },
    Rev16 {
        rd: u8,
        rm: u8,
    },
    Revsh {
        rd: u8,
        rm: u8,
    },
    /// SXT*/UXT* (rn = 15) and SXTA*/UXTA*; `bits` 8, 16, or 0 for the dual-halfword B16 forms.
    Extend {
        signed: bool,
        bits: u8,
        rd: u8,
        rn: u8,
        rm: u8,
        rot: u8,
    },
    /// BFI, or BFC when rn = 15.
    Bfi {
        rd: u8,
        rn: u8,
        lsb: u8,
        msb: u8,
    },
    Bfx {
        signed: bool,
        rd: u8,
        rn: u8,
        lsb: u8,
        widthm1: u8,
    },
    /// PKHBT (tb = false, LSL) / PKHTB (tb = true, ASR).
    Pkh {
        tb: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        shift: Shift,
    },
    /// Branch to PC + imm; cond 14 = always.
    B {
        cond: u8,
        imm: i32,
    },
    Bl {
        imm: i32,
    },
    Bx {
        rm: u8,
    },
    Blx {
        rm: u8,
    },
    Cbz {
        nonzero: bool,
        rn: u8,
        imm: u32,
    },
    Tb {
        half: bool,
        rn: u8,
        rm: u8,
    },
    /// Load; `size` 1, 2 or 4 bytes.
    Ldr {
        size: u8,
        signed: bool,
        unpriv: bool,
        rt: u8,
        rn: u8,
        addr: Addr,
    },
    Str {
        size: u8,
        unpriv: bool,
        rt: u8,
        rn: u8,
        addr: Addr,
    },
    Ldrd {
        rt: u8,
        rt2: u8,
        rn: u8,
        imm: u32,
        add: bool,
        index: bool,
        wback: bool,
    },
    Strd {
        rt: u8,
        rt2: u8,
        rn: u8,
        imm: u32,
        add: bool,
        index: bool,
        wback: bool,
    },
    /// LDM/POP: increment after (`db` = false) or decrement before.
    Ldm {
        rn: u8,
        regs: u16,
        wback: bool,
        db: bool,
    },
    Stm {
        rn: u8,
        regs: u16,
        wback: bool,
        db: bool,
    },
    Ldrex {
        size: u8,
        rt: u8,
        rn: u8,
        imm: u32,
    },
    Strex {
        size: u8,
        rd: u8,
        rt: u8,
        rn: u8,
        imm: u32,
    },
    Clrex,
    /// PLD/PLI and other no-operation memory hints.
    MemHint,
    Mrs {
        rd: u8,
        sysm: u8,
    },
    Msr {
        rn: u8,
        sysm: u8,
        mask: u8,
    },
    Cps {
        disable: bool,
        i: bool,
        f: bool,
    },
    Barrier(Barrier),
    Hint(Hint),
    It {
        firstcond: u8,
        mask: u8,
    },
    Bkpt {
        imm: u8,
    },
    Svc {
        imm: u8,
    },
    Vfp(Vfp),
}

/// True when the halfword is the first half of a 32-bit instruction.
#[inline]
pub fn is_32bit(hw1: u16) -> bool {
    (hw1 >> 11) >= 0b11101
}

/// Decodes one instruction. `hw2` is ignored for 16-bit instructions.
pub fn decode(hw1: u16, hw2: u16) -> Instr {
    if is_32bit(hw1) {
        decode32(hw1 as u32, hw2 as u32)
    } else {
        decode16(hw1 as u32)
    }
}

fn r3(h: u32, pos: u32) -> u8 {
    ((h >> pos) & 7) as u8
}

fn r4(h: u32, pos: u32) -> u8 {
    ((h >> pos) & 15) as u8
}

fn sign_extend(v: u32, bits: u32) -> i32 {
    let s = 32 - bits;
    ((v << s) as i32) >> s
}

/// `ThumbExpandImm_C` without the carry input: returns the value and its carry behaviour.
pub fn thumb_expand_imm(imm12: u32) -> (u32, ImmCarry) {
    if imm12 >> 10 == 0 {
        let b = imm12 & 0xFF;
        let v = match (imm12 >> 8) & 3 {
            0 => b,
            1 => (b << 16) | b,
            2 => (b << 24) | (b << 8),
            _ => (b << 24) | (b << 16) | (b << 8) | b,
        };
        (v, ImmCarry::Keep)
    } else {
        let unrot = 0x80 | (imm12 & 0x7F);
        let v = unrot.rotate_right(imm12 >> 7);
        (v, if v >> 31 != 0 { ImmCarry::Set } else { ImmCarry::Clear })
    }
}

fn decode16(h: u32) -> Instr {
    use Instr::*;
    match h >> 10 {
        0b000000..=0b001111 => {
            let op = (h >> 9) & 0x1F;
            let imm5 = (h >> 6) & 31;
            let rm = r3(h, 3);
            let rd = r3(h, 0);
            match op {
                0b00000..=0b00011 => DpReg { op: DpOp::Mov, s: SetFlags::OutsideIt, rd, rn: 0, rm, shift: Shift::imm(0, imm5) },
                0b00100..=0b00111 => DpReg { op: DpOp::Mov, s: SetFlags::OutsideIt, rd, rn: 0, rm, shift: Shift::imm(1, imm5) },
                0b01000..=0b01011 => DpReg { op: DpOp::Mov, s: SetFlags::OutsideIt, rd, rn: 0, rm, shift: Shift::imm(2, imm5) },
                0b01100 | 0b01101 => DpReg {
                    op: if op == 0b01100 { DpOp::Add } else { DpOp::Sub },
                    s: SetFlags::OutsideIt,
                    rd,
                    rn: r3(h, 3),
                    rm: r3(h, 6),
                    shift: Shift::NONE,
                },
                0b01110 | 0b01111 => DpImm {
                    op: if op == 0b01110 { DpOp::Add } else { DpOp::Sub },
                    s: SetFlags::OutsideIt,
                    rd,
                    rn: r3(h, 3),
                    imm: (h >> 6) & 7,
                    carry: ImmCarry::Keep,
                },
                _ => {
                    let rdn = r3(h, 8);
                    let imm = h & 0xFF;
                    match (op >> 2) & 3 {
                        0 => DpImm { op: DpOp::Mov, s: SetFlags::OutsideIt, rd: rdn, rn: 0, imm, carry: ImmCarry::Keep },
                        1 => DpImm { op: DpOp::Cmp, s: SetFlags::Yes, rd: 0, rn: rdn, imm, carry: ImmCarry::Keep },
                        2 => DpImm { op: DpOp::Add, s: SetFlags::OutsideIt, rd: rdn, rn: rdn, imm, carry: ImmCarry::Keep },
                        _ => DpImm { op: DpOp::Sub, s: SetFlags::OutsideIt, rd: rdn, rn: rdn, imm, carry: ImmCarry::Keep },
                    }
                }
            }
        }
        0b010000 => {
            let rdn = r3(h, 0);
            let rm = r3(h, 3);
            let s = SetFlags::OutsideIt;
            let dp = |op| DpReg { op, s, rd: rdn, rn: rdn, rm, shift: Shift::NONE };
            match (h >> 6) & 0xF {
                0b0000 => dp(DpOp::And),
                0b0001 => dp(DpOp::Eor),
                0b0010 => ShiftReg { s, ty: LSL, rd: rdn, rn: rdn, rm },
                0b0011 => ShiftReg { s, ty: LSR, rd: rdn, rn: rdn, rm },
                0b0100 => ShiftReg { s, ty: ASR, rd: rdn, rn: rdn, rm },
                0b0101 => dp(DpOp::Adc),
                0b0110 => dp(DpOp::Sbc),
                0b0111 => ShiftReg { s, ty: ROR, rd: rdn, rn: rdn, rm },
                0b1000 => DpReg { op: DpOp::Tst, s: SetFlags::Yes, rd: 0, rn: rdn, rm, shift: Shift::NONE },
                0b1001 => DpImm { op: DpOp::Rsb, s, rd: rdn, rn: rm, imm: 0, carry: ImmCarry::Keep },
                0b1010 => DpReg { op: DpOp::Cmp, s: SetFlags::Yes, rd: 0, rn: rdn, rm, shift: Shift::NONE },
                0b1011 => DpReg { op: DpOp::Cmn, s: SetFlags::Yes, rd: 0, rn: rdn, rm, shift: Shift::NONE },
                0b1100 => dp(DpOp::Orr),
                0b1101 => Mul { s, rd: rdn, rn: rm, rm: rdn },
                0b1110 => dp(DpOp::Bic),
                _ => DpReg { op: DpOp::Mvn, s, rd: rdn, rn: 0, rm, shift: Shift::NONE },
            }
        }
        0b010001 => {
            let rdn = (((h >> 7) & 1) << 3) as u8 | r3(h, 0);
            let rm = r4(h, 3);
            match (h >> 6) & 0xF {
                0b0000..=0b0011 => DpReg { op: DpOp::Add, s: SetFlags::No, rd: rdn, rn: rdn, rm, shift: Shift::NONE },
                0b0100..=0b0111 => DpReg { op: DpOp::Cmp, s: SetFlags::Yes, rd: 0, rn: rdn, rm, shift: Shift::NONE },
                0b1000..=0b1011 => DpReg { op: DpOp::Mov, s: SetFlags::No, rd: rdn, rn: 0, rm, shift: Shift::NONE },
                0b1100 | 0b1101 => Bx { rm },
                _ => Blx { rm },
            }
        }
        0b010010 | 0b010011 => {
            Ldr { size: 4, signed: false, unpriv: false, rt: r3(h, 8), rn: 15, addr: Addr::Literal { imm: (h & 0xFF) << 2, add: true } }
        }
        0b010100..=0b010111 => {
            let addr = Addr::Reg { rm: r3(h, 6), shift: 0 };
            let (rt, rn) = (r3(h, 0), r3(h, 3));
            match (h >> 9) & 7 {
                0 => Str { size: 4, unpriv: false, rt, rn, addr },
                1 => Str { size: 2, unpriv: false, rt, rn, addr },
                2 => Str { size: 1, unpriv: false, rt, rn, addr },
                3 => Ldr { size: 1, signed: true, unpriv: false, rt, rn, addr },
                4 => Ldr { size: 4, signed: false, unpriv: false, rt, rn, addr },
                5 => Ldr { size: 2, signed: false, unpriv: false, rt, rn, addr },
                6 => Ldr { size: 1, signed: false, unpriv: false, rt, rn, addr },
                _ => Ldr { size: 2, signed: true, unpriv: false, rt, rn, addr },
            }
        }
        0b011000..=0b100011 => {
            let size: u8 = match h >> 11 {
                0b01100 | 0b01101 => 4,
                0b01110 | 0b01111 => 1,
                _ => 2,
            };
            let imm = ((h >> 6) & 31) * size as u32;
            let addr = Addr::Imm { imm, add: true, index: true, wback: false };
            let (rt, rn) = (r3(h, 0), r3(h, 3));
            if (h >> 11) & 1 == 1 {
                Ldr { size, signed: false, unpriv: false, rt, rn, addr }
            } else {
                Str { size, unpriv: false, rt, rn, addr }
            }
        }
        0b100100..=0b100111 => {
            let addr = Addr::Imm { imm: (h & 0xFF) << 2, add: true, index: true, wback: false };
            let rt = r3(h, 8);
            if (h >> 11) & 1 == 1 {
                Ldr { size: 4, signed: false, unpriv: false, rt, rn: 13, addr }
            } else {
                Str { size: 4, unpriv: false, rt, rn: 13, addr }
            }
        }
        0b101000 | 0b101001 => Adr { rd: r3(h, 8), imm: (h & 0xFF) << 2, add: true },
        0b101010 | 0b101011 => DpImm { op: DpOp::Add, s: SetFlags::No, rd: r3(h, 8), rn: 13, imm: (h & 0xFF) << 2, carry: ImmCarry::Keep },
        0b101100..=0b101111 => decode16_misc(h),
        0b110000 | 0b110001 => Stm { rn: r3(h, 8), regs: (h & 0xFF) as u16, wback: true, db: false },
        0b110010 | 0b110011 => {
            let rn = r3(h, 8);
            let regs = (h & 0xFF) as u16;
            Ldm { rn, regs, wback: regs & (1 << rn) == 0, db: false }
        }
        0b110100..=0b110111 => {
            let cond = ((h >> 8) & 0xF) as u8;
            match cond {
                0xE => Undefined,
                0xF => Svc { imm: (h & 0xFF) as u8 },
                _ => B { cond, imm: sign_extend((h & 0xFF) << 1, 9) },
            }
        }
        0b111000 | 0b111001 => B { cond: 14, imm: sign_extend((h & 0x7FF) << 1, 12) },
        _ => Undefined,
    }
}

fn decode16_misc(h: u32) -> Instr {
    use Instr::*;
    if h & 0xFFEC == 0xB660 {
        return Cps { disable: (h >> 4) & 1 == 1, i: (h >> 1) & 1 == 1, f: h & 1 == 1 };
    }
    if h & 0xFF80 == 0xB000 {
        return DpImm { op: DpOp::Add, s: SetFlags::No, rd: 13, rn: 13, imm: (h & 0x7F) << 2, carry: ImmCarry::Keep };
    }
    if h & 0xFF80 == 0xB080 {
        return DpImm { op: DpOp::Sub, s: SetFlags::No, rd: 13, rn: 13, imm: (h & 0x7F) << 2, carry: ImmCarry::Keep };
    }
    if h & 0xF500 == 0xB100 {
        let imm = (((h >> 9) & 1) << 6) | (((h >> 3) & 31) << 1);
        return Cbz { nonzero: (h >> 11) & 1 == 1, rn: r3(h, 0), imm };
    }
    match h & 0xFFC0 {
        0xB200 => return Extend { signed: true, bits: 16, rd: r3(h, 0), rn: 15, rm: r3(h, 3), rot: 0 },
        0xB240 => return Extend { signed: true, bits: 8, rd: r3(h, 0), rn: 15, rm: r3(h, 3), rot: 0 },
        0xB280 => return Extend { signed: false, bits: 16, rd: r3(h, 0), rn: 15, rm: r3(h, 3), rot: 0 },
        0xB2C0 => return Extend { signed: false, bits: 8, rd: r3(h, 0), rn: 15, rm: r3(h, 3), rot: 0 },
        0xBA00 => return Rev { rd: r3(h, 0), rm: r3(h, 3) },
        0xBA40 => return Rev16 { rd: r3(h, 0), rm: r3(h, 3) },
        0xBAC0 => return Revsh { rd: r3(h, 0), rm: r3(h, 3) },
        _ => {}
    }
    if h & 0xFE00 == 0xB400 {
        let regs = (h & 0xFF) as u16 | ((((h >> 8) & 1) as u16) << 14);
        return Stm { rn: 13, regs, wback: true, db: true };
    }
    if h & 0xFE00 == 0xBC00 {
        let regs = (h & 0xFF) as u16 | ((((h >> 8) & 1) as u16) << 15);
        return Ldm { rn: 13, regs, wback: true, db: false };
    }
    if h & 0xFF00 == 0xBE00 {
        return Bkpt { imm: (h & 0xFF) as u8 };
    }
    if h & 0xFF00 == 0xBF00 {
        let mask = (h & 0xF) as u8;
        if mask != 0 {
            return It { firstcond: ((h >> 4) & 0xF) as u8, mask };
        }
        return Hint(match (h >> 4) & 0xF {
            1 => self::Hint::Yield,
            2 => self::Hint::Wfe,
            3 => self::Hint::Wfi,
            4 => self::Hint::Sev,
            _ => self::Hint::Nop,
        });
    }
    Undefined
}

fn decode32(h1: u32, h2: u32) -> Instr {
    let op1 = (h1 >> 11) & 3;
    let op2 = (h1 >> 4) & 0x7F;
    match op1 {
        0b01 => {
            if op2 & 0b1100100 == 0b0000000 {
                decode_ldm_stm(h1, h2)
            } else if op2 & 0b1100100 == 0b0000100 {
                decode_dual_excl(h1, h2)
            } else if op2 & 0b1100000 == 0b0100000 {
                decode_dp_shifted(h1, h2)
            } else {
                decode_coproc(h1, h2)
            }
        }
        0b10 => {
            if h2 & 0x8000 == 0 {
                if op2 & 0b0100000 == 0 {
                    decode_dp_modimm(h1, h2)
                } else {
                    decode_dp_plainimm(h1, h2)
                }
            } else {
                decode_branch_misc(h1, h2)
            }
        }
        _ => {
            if op2 & 0b1000000 != 0 {
                decode_coproc(h1, h2)
            } else if op2 & 0b1110001 == 0b0000000
                || op2 & 0b1100111 == 0b0000001
                || op2 & 0b1100111 == 0b0000011
                || op2 & 0b1100111 == 0b0000101
            {
                decode_ldst_single(h1, h2)
            } else if op2 & 0b1110000 == 0b0100000 {
                decode_dp_register(h1, h2)
            } else if op2 & 0b1111000 == 0b0110000 {
                decode_multiply(h1, h2)
            } else if op2 & 0b1111000 == 0b0111000 {
                decode_long_multiply(h1, h2)
            } else {
                Instr::Undefined
            }
        }
    }
}

fn decode_ldm_stm(h1: u32, h2: u32) -> Instr {
    let op = (h1 >> 7) & 3;
    let w = (h1 >> 5) & 1 == 1;
    let l = (h1 >> 4) & 1 == 1;
    let rn = r4(h1, 0);
    let regs = h2 as u16;
    let db = match op {
        0b01 => false,
        0b10 => true,
        _ => return Instr::Undefined,
    };
    if l {
        Instr::Ldm { rn, regs, wback: w, db }
    } else {
        Instr::Stm { rn, regs, wback: w, db }
    }
}

fn decode_dual_excl(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    let p = (h1 >> 8) & 1;
    let u = (h1 >> 7) & 1;
    let w = (h1 >> 5) & 1;
    let l = (h1 >> 4) & 1;
    let rn = r4(h1, 0);
    let rt = r4(h2, 12);
    if p == 0 && w == 0 {
        let op3 = (h2 >> 4) & 0xF;
        match (u, l) {
            (0, 0) => Strex { size: 4, rd: r4(h2, 8), rt, rn, imm: (h2 & 0xFF) << 2 },
            (0, _) => Ldrex { size: 4, rt, rn, imm: (h2 & 0xFF) << 2 },
            (_, 0) => match op3 {
                0b0100 => Strex { size: 1, rd: r4(h2, 0), rt, rn, imm: 0 },
                0b0101 => Strex { size: 2, rd: r4(h2, 0), rt, rn, imm: 0 },
                _ => Undefined,
            },
            _ => match op3 {
                0b0000 => Tb { half: false, rn, rm: r4(h2, 0) },
                0b0001 => Tb { half: true, rn, rm: r4(h2, 0) },
                0b0100 => Ldrex { size: 1, rt, rn, imm: 0 },
                0b0101 => Ldrex { size: 2, rt, rn, imm: 0 },
                _ => Undefined,
            },
        }
    } else {
        let rt2 = r4(h2, 8);
        let imm = (h2 & 0xFF) << 2;
        let (add, index, wback) = (u == 1, p == 1, w == 1);
        if l == 1 {
            Ldrd { rt, rt2, rn, imm, add, index, wback }
        } else {
            Strd { rt, rt2, rn, imm, add, index, wback }
        }
    }
}

fn decode_dp_shifted(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    let op = (h1 >> 5) & 0xF;
    let s = if (h1 >> 4) & 1 == 1 { SetFlags::Yes } else { SetFlags::No };
    let rn = r4(h1, 0);
    let rd = r4(h2, 8);
    let rm = r4(h2, 0);
    let imm5 = (((h2 >> 12) & 7) << 2) | ((h2 >> 6) & 3);
    let ty = (h2 >> 4) & 3;
    let shift = Shift::imm(ty, imm5);
    let test = rd == 15 && s == SetFlags::Yes;
    let dp = |op| DpReg { op, s, rd, rn, rm, shift };
    match op {
        0b0000 => {
            if test {
                DpReg { op: DpOp::Tst, s, rd: 0, rn, rm, shift }
            } else {
                dp(DpOp::And)
            }
        }
        0b0001 => dp(DpOp::Bic),
        0b0010 => {
            if rn == 15 {
                DpReg { op: DpOp::Mov, s, rd, rn: 0, rm, shift }
            } else {
                dp(DpOp::Orr)
            }
        }
        0b0011 => {
            if rn == 15 {
                DpReg { op: DpOp::Mvn, s, rd, rn: 0, rm, shift }
            } else {
                dp(DpOp::Orn)
            }
        }
        0b0100 => {
            if test {
                DpReg { op: DpOp::Teq, s, rd: 0, rn, rm, shift }
            } else {
                dp(DpOp::Eor)
            }
        }
        0b0110 => {
            if s == SetFlags::Yes || ty & 1 == 1 {
                Undefined
            } else {
                Pkh { tb: ty == 2, rd, rn, rm, shift }
            }
        }
        0b1000 => {
            if test {
                DpReg { op: DpOp::Cmn, s, rd: 0, rn, rm, shift }
            } else {
                dp(DpOp::Add)
            }
        }
        0b1010 => dp(DpOp::Adc),
        0b1011 => dp(DpOp::Sbc),
        0b1101 => {
            if test {
                DpReg { op: DpOp::Cmp, s, rd: 0, rn, rm, shift }
            } else {
                dp(DpOp::Sub)
            }
        }
        0b1110 => dp(DpOp::Rsb),
        _ => Undefined,
    }
}

fn decode_dp_modimm(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    let op = (h1 >> 5) & 0xF;
    let s = if (h1 >> 4) & 1 == 1 { SetFlags::Yes } else { SetFlags::No };
    let rn = r4(h1, 0);
    let rd = r4(h2, 8);
    let imm12 = (((h1 >> 10) & 1) << 11) | (((h2 >> 12) & 7) << 8) | (h2 & 0xFF);
    let (imm, carry) = thumb_expand_imm(imm12);
    let test = rd == 15 && s == SetFlags::Yes;
    let dp = |op| DpImm { op, s, rd, rn, imm, carry };
    match op {
        0b0000 => {
            if test {
                DpImm { op: DpOp::Tst, s, rd: 0, rn, imm, carry }
            } else {
                dp(DpOp::And)
            }
        }
        0b0001 => dp(DpOp::Bic),
        0b0010 => {
            if rn == 15 {
                DpImm { op: DpOp::Mov, s, rd, rn: 0, imm, carry }
            } else {
                dp(DpOp::Orr)
            }
        }
        0b0011 => {
            if rn == 15 {
                DpImm { op: DpOp::Mvn, s, rd, rn: 0, imm, carry }
            } else {
                dp(DpOp::Orn)
            }
        }
        0b0100 => {
            if test {
                DpImm { op: DpOp::Teq, s, rd: 0, rn, imm, carry }
            } else {
                dp(DpOp::Eor)
            }
        }
        0b1000 => {
            if test {
                DpImm { op: DpOp::Cmn, s, rd: 0, rn, imm, carry }
            } else {
                dp(DpOp::Add)
            }
        }
        0b1010 => dp(DpOp::Adc),
        0b1011 => dp(DpOp::Sbc),
        0b1101 => {
            if test {
                DpImm { op: DpOp::Cmp, s, rd: 0, rn, imm, carry }
            } else {
                dp(DpOp::Sub)
            }
        }
        0b1110 => dp(DpOp::Rsb),
        _ => Undefined,
    }
}

fn decode_dp_plainimm(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    let op = (h1 >> 4) & 0x1F;
    let rn = r4(h1, 0);
    let rd = r4(h2, 8);
    let imm12 = (((h1 >> 10) & 1) << 11) | (((h2 >> 12) & 7) << 8) | (h2 & 0xFF);
    let imm16 = ((h1 & 0xF) << 12) | imm12;
    let imm3_2 = (((h2 >> 12) & 7) << 2) | ((h2 >> 6) & 3);
    let low5 = (h2 & 0x1F) as u8;
    match op {
        0b00000 => {
            if rn == 15 {
                Adr { rd, imm: imm12, add: true }
            } else {
                DpImm { op: DpOp::Add, s: SetFlags::No, rd, rn, imm: imm12, carry: ImmCarry::Keep }
            }
        }
        0b00100 => DpImm { op: DpOp::Mov, s: SetFlags::No, rd, rn: 0, imm: imm16, carry: ImmCarry::Keep },
        0b01010 => {
            if rn == 15 {
                Adr { rd, imm: imm12, add: false }
            } else {
                DpImm { op: DpOp::Sub, s: SetFlags::No, rd, rn, imm: imm12, carry: ImmCarry::Keep }
            }
        }
        0b01100 => MovT { rd, imm: imm16 as u16 },
        0b10000 | 0b10010 => {
            if op == 0b10010 && imm3_2 == 0 {
                Sat16 { signed: true, rd, rn, sat: (h2 & 0xF) as u8 + 1 }
            } else {
                Sat { signed: true, rd, rn, sat: low5 + 1, shift: Shift::imm(if (op >> 1) & 1 == 1 { 2 } else { 0 }, imm3_2) }
            }
        }
        0b10100 => Bfx { signed: true, rd, rn, lsb: imm3_2 as u8, widthm1: low5 },
        0b10110 => Bfi { rd, rn, lsb: imm3_2 as u8, msb: low5 },
        0b11000 | 0b11010 => {
            if op == 0b11010 && imm3_2 == 0 {
                Sat16 { signed: false, rd, rn, sat: (h2 & 0xF) as u8 }
            } else {
                Sat { signed: false, rd, rn, sat: low5, shift: Shift::imm(if (op >> 1) & 1 == 1 { 2 } else { 0 }, imm3_2) }
            }
        }
        0b11100 => Bfx { signed: false, rd, rn, lsb: imm3_2 as u8, widthm1: low5 },
        _ => Undefined,
    }
}

fn decode_branch_misc(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    let op1 = (h2 >> 12) & 7;
    let s = (h1 >> 10) & 1;
    let j1 = (h2 >> 13) & 1;
    let j2 = (h2 >> 11) & 1;
    if op1 & 0b101 == 0b000 {
        if (h1 >> 7) & 7 != 7 {
            let imm = (s << 20) | (j2 << 19) | (j1 << 18) | ((h1 & 0x3F) << 12) | ((h2 & 0x7FF) << 1);
            return B { cond: ((h1 >> 6) & 0xF) as u8, imm: sign_extend(imm, 21) };
        }
        let op = (h1 >> 4) & 0x7F;
        return match op {
            0b0111000 | 0b0111001 => Msr { rn: r4(h1, 0), sysm: (h2 & 0xFF) as u8, mask: ((h2 >> 10) & 3) as u8 },
            0b0111010 => {
                if (h2 >> 8) & 7 != 0 {
                    Undefined
                } else {
                    Hint(match h2 & 0xFF {
                        1 => self::Hint::Yield,
                        2 => self::Hint::Wfe,
                        3 => self::Hint::Wfi,
                        4 => self::Hint::Sev,
                        _ => self::Hint::Nop,
                    })
                }
            }
            0b0111011 => match (h2 >> 4) & 0xF {
                0b0010 => Clrex,
                0b0100 => Barrier(self::Barrier::Dsb),
                0b0101 => Barrier(self::Barrier::Dmb),
                0b0110 => Barrier(self::Barrier::Isb),
                _ => Undefined,
            },
            0b0111110 | 0b0111111 => Mrs { rd: r4(h2, 8), sysm: (h2 & 0xFF) as u8 },
            _ => Undefined,
        };
    }
    if op1 == 0b010 && (h1 >> 4) & 0x7F == 0x7F {
        return Undefined;
    }
    let i1 = !(j1 ^ s) & 1;
    let i2 = !(j2 ^ s) & 1;
    let imm = (s << 24) | (i1 << 23) | (i2 << 22) | ((h1 & 0x3FF) << 12) | ((h2 & 0x7FF) << 1);
    let imm = sign_extend(imm, 25);
    match op1 & 0b101 {
        0b001 => B { cond: 14, imm },
        0b101 => Bl { imm },
        _ => Undefined,
    }
}

fn decode_ldst_single(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    let signed = (h1 >> 8) & 1 == 1;
    let u12 = (h1 >> 7) & 1 == 1;
    let size: u8 = match (h1 >> 5) & 3 {
        0 => 1,
        1 => 2,
        2 => 4,
        _ => return Undefined,
    };
    let load = (h1 >> 4) & 1 == 1;
    let rn = r4(h1, 0);
    let rt = r4(h2, 12);
    if signed && (!load || size == 4) {
        return Undefined;
    }
    let mut unpriv = false;
    let addr = if rn == 15 {
        if !load {
            return Undefined;
        }
        Addr::Literal { imm: h2 & 0xFFF, add: u12 }
    } else if u12 {
        Addr::Imm { imm: h2 & 0xFFF, add: true, index: true, wback: false }
    } else if h2 & 0x800 != 0 {
        let p = (h2 >> 10) & 1 == 1;
        let u = (h2 >> 9) & 1 == 1;
        let w = (h2 >> 8) & 1 == 1;
        if p && u && !w {
            unpriv = true;
            Addr::Imm { imm: h2 & 0xFF, add: true, index: true, wback: false }
        } else if !p && !w {
            return Undefined;
        } else {
            Addr::Imm { imm: h2 & 0xFF, add: u, index: p, wback: w }
        }
    } else if (h2 >> 6) & 0x3F == 0 {
        Addr::Reg { rm: r4(h2, 0), shift: ((h2 >> 4) & 3) as u8 }
    } else {
        return Undefined;
    };
    if load {
        if rt == 15 && size != 4 {
            return MemHint;
        }
        Ldr { size, signed, unpriv, rt, rn, addr }
    } else {
        Str { size, unpriv, rt, rn, addr }
    }
}

fn decode_dp_register(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    if (h2 >> 12) & 0xF != 0xF {
        return Undefined;
    }
    let op1 = (h1 >> 4) & 0xF;
    let op2 = (h2 >> 4) & 0xF;
    let rn = r4(h1, 0);
    let rd = r4(h2, 8);
    let rm = r4(h2, 0);
    if op1 & 0b1000 == 0 && op2 == 0 {
        let s = if op1 & 1 == 1 { SetFlags::Yes } else { SetFlags::No };
        return ShiftReg { s, ty: ((op1 >> 1) & 3) as u8, rd, rn, rm };
    }
    if op1 & 0b1000 == 0 && op2 & 0b1000 != 0 {
        let rot = ((h2 >> 4) & 3) as u8 * 8;
        let (signed, bits) = match op1 {
            0b0000 => (true, 16),
            0b0001 => (false, 16),
            0b0010 => (true, 0),
            0b0011 => (false, 0),
            0b0100 => (true, 8),
            0b0101 => (false, 8),
            _ => return Undefined,
        };
        return Extend { signed, bits, rd, rn, rm, rot };
    }
    if op1 & 0b1000 != 0 && op2 & 0b1000 == 0 {
        let op = match op1 & 7 {
            0b000 => ParOp::Add8,
            0b001 => ParOp::Add16,
            0b010 => ParOp::Asx,
            0b100 => ParOp::Sub8,
            0b101 => ParOp::Sub16,
            0b110 => ParOp::Sax,
            _ => return Undefined,
        };
        let kind = match op2 & 7 {
            0b000 => ParKind::S,
            0b001 => ParKind::Q,
            0b010 => ParKind::Sh,
            0b100 => ParKind::U,
            0b101 => ParKind::Uq,
            0b110 => ParKind::Uh,
            _ => return Undefined,
        };
        return Parallel { op, kind, rd, rn, rm };
    }
    if op1 & 0b1100 == 0b1000 && op2 & 0b1100 == 0b1000 {
        return match (op1 & 3, op2 & 3) {
            (0, 0) => Qop { op: QOp::Qadd, rd, rn, rm },
            (0, 1) => Qop { op: QOp::Qdadd, rd, rn, rm },
            (0, 2) => Qop { op: QOp::Qsub, rd, rn, rm },
            (0, 3) => Qop { op: QOp::Qdsub, rd, rn, rm },
            (1, 0) => Rev { rd, rm },
            (1, 1) => Rev16 { rd, rm },
            (1, 2) => Rbit { rd, rm },
            (1, 3) => Revsh { rd, rm },
            (2, 0) => Sel { rd, rn, rm },
            (3, 0) => Clz { rd, rm },
            _ => Undefined,
        };
    }
    Undefined
}

fn decode_multiply(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    if (h2 >> 6) & 3 != 0 {
        return Undefined;
    }
    let op1 = (h1 >> 4) & 7;
    let op2 = (h2 >> 4) & 3;
    let rn = r4(h1, 0);
    let ra = r4(h2, 12);
    let rd = r4(h2, 8);
    let rm = r4(h2, 0);
    match op1 {
        0 => match op2 {
            0 => {
                if ra == 15 {
                    Mul { s: SetFlags::No, rd, rn, rm }
                } else {
                    Mla { sub: false, rd, rn, rm, ra }
                }
            }
            1 => Mla { sub: true, rd, rn, rm, ra },
            _ => Undefined,
        },
        1 => SmlaXy { rd, rn, rm, ra, nh: op2 >> 1 == 1, mh: op2 & 1 == 1 },
        2 if op2 >> 1 == 0 => Smlad { sub: false, x: op2 & 1 == 1, rd, rn, rm, ra },
        3 if op2 >> 1 == 0 => SmlaWy { rd, rn, rm, ra, mh: op2 & 1 == 1 },
        4 if op2 >> 1 == 0 => Smlad { sub: true, x: op2 & 1 == 1, rd, rn, rm, ra },
        5 if op2 >> 1 == 0 => Smmla { sub: false, round: op2 & 1 == 1, rd, rn, rm, ra },
        6 if op2 >> 1 == 0 => Smmla { sub: true, round: op2 & 1 == 1, rd, rn, rm, ra },
        7 if op2 == 0 => Usada8 { rd, rn, rm, ra },
        _ => Undefined,
    }
}

fn decode_long_multiply(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    let op1 = (h1 >> 4) & 7;
    let op2 = (h2 >> 4) & 0xF;
    let rn = r4(h1, 0);
    let rdlo = r4(h2, 12);
    let rdhi = r4(h2, 8);
    let rm = r4(h2, 0);
    match (op1, op2) {
        (0, 0) => LongMul { op: self::LongMul::Smull, rdlo, rdhi, rn, rm },
        (1, 0xF) => Div { signed: true, rd: rdhi, rn, rm },
        (2, 0) => LongMul { op: self::LongMul::Umull, rdlo, rdhi, rn, rm },
        (3, 0xF) => Div { signed: false, rd: rdhi, rn, rm },
        (4, 0) => LongMul { op: self::LongMul::Smlal, rdlo, rdhi, rn, rm },
        (4, 0b1000..=0b1011) => SmlalXy { rdlo, rdhi, rn, rm, nh: (op2 >> 1) & 1 == 1, mh: op2 & 1 == 1 },
        (4, 0b1100 | 0b1101) => Smlald { sub: false, x: op2 & 1 == 1, rdlo, rdhi, rn, rm },
        (5, 0b1100 | 0b1101) => Smlald { sub: true, x: op2 & 1 == 1, rdlo, rdhi, rn, rm },
        (6, 0) => LongMul { op: self::LongMul::Umlal, rdlo, rdhi, rn, rm },
        (6, 0b0110) => LongMul { op: self::LongMul::Umaal, rdlo, rdhi, rn, rm },
        _ => Undefined,
    }
}

/// `VFPExpandImm` for single precision.
pub fn vfp_expand_imm(imm8: u32) -> u32 {
    let sign = (imm8 >> 7) & 1;
    let b6 = (imm8 >> 6) & 1;
    let exp = ((b6 ^ 1) << 7) | (if b6 == 1 { 0b11111 << 2 } else { 0 }) | ((imm8 >> 4) & 3);
    let frac = (imm8 & 0xF) << 19;
    (sign << 31) | (exp << 23) | frac
}

fn decode_coproc(h1: u32, h2: u32) -> Instr {
    use Instr::*;
    let coproc = (h2 >> 8) & 0xF;
    if coproc & 0b1110 != 0b1010 {
        return NoCoproc;
    }
    if (h1 >> 12) & 1 == 1 {
        return Undefined;
    }
    let double = coproc & 1 == 1;
    let op1 = (h1 >> 4) & 0x3F;
    if op1 & 0b100000 == 0 {
        // extension register load/store and 64-bit transfers
        let p = (h1 >> 8) & 1 == 1;
        let u = (h1 >> 7) & 1 == 1;
        let d = (h1 >> 6) & 1;
        let w = (h1 >> 5) & 1 == 1;
        let l = (h1 >> 4) & 1 == 1;
        let rn = r4(h1, 0);
        let vd = (h2 >> 12) & 0xF;
        let imm8 = h2 & 0xFF;
        if !p && !u {
            if d == 1 && !w {
                let to_core = l;
                let rt = r4(h2, 12);
                let rt2 = r4(h1, 0);
                let vm = h2 & 0xF;
                let m = (h2 >> 5) & 1;
                let sm = if double { (m << 4 | vm) * 2 } else { vm << 1 | m };
                if (h2 >> 4) & 0b1101 != 0b0001 {
                    return Undefined;
                }
                return Vfp(self::Vfp::MovCore2 { to_core, rt, rt2, sm: sm as u8 });
            }
            return Undefined;
        }
        let sd = if double { (d << 4 | vd) * 2 } else { vd << 1 | d } as u8;
        if p && !w {
            return Vfp(self::Vfp::LdrStr { load: l, double, sd, rn, imm: imm8 << 2, add: u });
        }
        let count = if double { (imm8 / 2) * 2 } else { imm8 } as u8;
        if !p && u {
            return Vfp(self::Vfp::LdmStm { load: l, sd, count, rn, inc: true, wback: w });
        }
        if p && !u && w {
            return Vfp(self::Vfp::LdmStm { load: l, sd, count, rn, inc: false, wback: true });
        }
        return Undefined;
    }
    if op1 & 0b110000 != 0b100000 {
        return Undefined;
    }
    if h2 & 0x10 == 0 {
        if double {
            return Undefined;
        }
        return decode_vfp_dp(h1, h2);
    }
    // 8, 16 and 32-bit transfers between core and extension registers
    let a = (h1 >> 5) & 7;
    let l = (h1 >> 4) & 1 == 1;
    let c = (h2 >> 8) & 1;
    let rt = r4(h2, 12);
    if c == 0 {
        match a {
            0b000 => {
                let sn = ((h1 & 0xF) << 1 | (h2 >> 7) & 1) as u8;
                Vfp(self::Vfp::MovCore { to_core: l, rt, sn })
            }
            0b111 => {
                if h1 & 0xF != 1 {
                    // only FPSCR is accessible
                    return Undefined;
                }
                if l {
                    Vfp(self::Vfp::Vmrs { rt })
                } else {
                    Vfp(self::Vfp::Vmsr { rt })
                }
            }
            _ => Undefined,
        }
    } else {
        // VMOV between a core register and a 32-bit scalar
        if (h1 >> 6) & 1 != 0 || (h2 >> 5) & 3 != 0 {
            return Undefined;
        }
        let x = (h1 >> 5) & 1;
        let dd = ((h2 >> 7) & 1) << 4 | (h1 & 0xF);
        Vfp(self::Vfp::MovScalar { to_core: l, rt, sm: (dd * 2 + x) as u8 })
    }
}

fn decode_vfp_dp(h1: u32, h2: u32) -> Instr {
    use self::Vfp::*;
    let opc1 = (h1 >> 4) & 0b1011;
    let opc2 = h1 & 0xF;
    let opc3 = (h2 >> 6) & 3;
    let op = (h2 >> 6) & 1 == 1;
    let sd = (((h2 >> 12) & 0xF) << 1 | (h1 >> 6) & 1) as u8;
    let sn = ((h1 & 0xF) << 1 | (h2 >> 7) & 1) as u8;
    let sm = ((h2 & 0xF) << 1 | (h2 >> 5) & 1) as u8;
    let arith = |o| Instr::Vfp(Arith { op: o, sd, sn, sm });
    match opc1 {
        0b0000 => arith(if op { VArith::Mls } else { VArith::Mla }),
        0b0001 => arith(if op { VArith::Nmla } else { VArith::Nmls }),
        0b0010 => arith(if op { VArith::Nmul } else { VArith::Mul }),
        0b0011 => arith(if op { VArith::Sub } else { VArith::Add }),
        0b1000 => {
            if op {
                Instr::Undefined
            } else {
                arith(VArith::Div)
            }
        }
        0b1001 => arith(if op { VArith::Fnma } else { VArith::Fnms }),
        0b1010 => arith(if op { VArith::Fms } else { VArith::Fma }),
        0b1011 => {
            if opc3 & 1 == 0 {
                let imm8 = (opc2 << 4) | (h2 & 0xF);
                return Instr::Vfp(MovImm { sd, imm: vfp_expand_imm(imm8) });
            }
            let top = (h2 >> 7) & 1 == 1;
            Instr::Vfp(match (opc2, opc3) {
                (0b0000, 0b01) => Unary { op: VUnary::Mov, sd, sm },
                (0b0000, 0b11) => Unary { op: VUnary::Abs, sd, sm },
                (0b0001, 0b01) => Unary { op: VUnary::Neg, sd, sm },
                (0b0001, 0b11) => Unary { op: VUnary::Sqrt, sd, sm },
                (0b0010 | 0b0011, _) => CvtHalf { to_half: opc2 & 1 == 1, top, sd, sm },
                (0b0100, _) => Cmp { e: top, zero: false, sd, sm },
                (0b0101, _) => Cmp { e: top, zero: true, sd, sm },
                (0b1000, _) => CvtFromInt { signed: top, sd, sm },
                (0b1100, _) => CvtToInt { signed: false, round_zero: top, sd, sm },
                (0b1101, _) => CvtToInt { signed: true, round_zero: top, sd, sm },
                (0b1010 | 0b1011 | 0b1110 | 0b1111, _) => {
                    let bits: u8 = if top { 32 } else { 16 };
                    let imm5 = ((h2 & 0xF) << 1 | (h2 >> 5) & 1) as u8;
                    if imm5 > bits {
                        return Instr::Undefined;
                    }
                    CvtFixed { to_fixed: (opc2 >> 2) & 1 == 1, signed: opc2 & 1 == 0, bits, frac: bits - imm5, sd }
                }
                _ => return Instr::Undefined,
            })
        }
        _ => Instr::Undefined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d16(h: u16) -> Instr {
        decode(h, 0)
    }

    #[test]
    fn sixteen_bit() {
        // movs r0, #1
        assert_eq!(d16(0x2001), Instr::DpImm { op: DpOp::Mov, s: SetFlags::OutsideIt, rd: 0, rn: 0, imm: 1, carry: ImmCarry::Keep });
        // push {r4, lr}
        assert_eq!(d16(0xB510), Instr::Stm { rn: 13, regs: 0x4010, wback: true, db: true });
        // pop {r4, pc}
        assert_eq!(d16(0xBD10), Instr::Ldm { rn: 13, regs: 0x8010, wback: true, db: false });
        // bx lr
        assert_eq!(d16(0x4770), Instr::Bx { rm: 14 });
        // ldr r3, [pc, #8]
        assert_eq!(
            d16(0x4B02),
            Instr::Ldr { size: 4, signed: false, unpriv: false, rt: 3, rn: 15, addr: Addr::Literal { imm: 8, add: true } }
        );
        // cbz r0, +4
        assert_eq!(d16(0xB110), Instr::Cbz { nonzero: false, rn: 0, imm: 4 });
        // it eq
        assert_eq!(d16(0xBF08), Instr::It { firstcond: 0, mask: 8 });
        // wfi
        assert_eq!(d16(0xBF30), Instr::Hint(Hint::Wfi));
        // b.n -4
        assert_eq!(d16(0xE7FE), Instr::B { cond: 14, imm: -4 });
        // cpsie i
        assert_eq!(d16(0xB662), Instr::Cps { disable: false, i: true, f: false });
        // svc 0
        assert_eq!(d16(0xDF00), Instr::Svc { imm: 0 });
    }

    #[test]
    fn thirty_two_bit() {
        // bl with offset 0 (f000 f800)
        assert_eq!(decode(0xF000, 0xF800), Instr::Bl { imm: 0 });
        // mov.w r0, #0x55555555 (f04f 3055)
        assert_eq!(
            decode(0xF04F, 0x3055),
            Instr::DpImm { op: DpOp::Mov, s: SetFlags::No, rd: 0, rn: 0, imm: 0x5555_5555, carry: ImmCarry::Keep }
        );
        // movw r3, #0x1234 (f241 2334)
        assert_eq!(
            decode(0xF241, 0x2334),
            Instr::DpImm { op: DpOp::Mov, s: SetFlags::No, rd: 3, rn: 0, imm: 0x1234, carry: ImmCarry::Keep }
        );
        // movt r3, #0x2000 (f2c2 0300)
        assert_eq!(decode(0xF2C2, 0x0300), Instr::MovT { rd: 3, imm: 0x2000 });
        // ldr.w r0, [r1, #4] (f8d1 0004)
        assert_eq!(
            decode(0xF8D1, 0x0004),
            Instr::Ldr {
                size: 4,
                signed: false,
                unpriv: false,
                rt: 0,
                rn: 1,
                addr: Addr::Imm { imm: 4, add: true, index: true, wback: false }
            }
        );
        // push.w {r4-r11, lr} (e92d 4ff0)
        assert_eq!(decode(0xE92D, 0x4FF0), Instr::Stm { rn: 13, regs: 0x4FF0, wback: true, db: true });
        // udiv r0, r0, r1 (fbb0 f0f1)
        assert_eq!(decode(0xFBB0, 0xF0F1), Instr::Div { signed: false, rd: 0, rn: 0, rm: 1 });
        // vadd.f32 s0, s0, s1 (ee30 0a20)
        assert_eq!(decode(0xEE30, 0x0A20), Instr::Vfp(Vfp::Arith { op: VArith::Add, sd: 0, sn: 0, sm: 1 }));
        // vmrs APSR_nzcv, fpscr (eef1 fa10)
        assert_eq!(decode(0xEEF1, 0xFA10), Instr::Vfp(Vfp::Vmrs { rt: 15 }));
        // vpush {s16-s31} (ed2d 8a10)
        assert_eq!(decode(0xED2D, 0x8A10), Instr::Vfp(Vfp::LdmStm { load: false, sd: 16, count: 16, rn: 13, inc: false, wback: true }));
        // mrs r0, primask (f3ef 8010)
        assert_eq!(decode(0xF3EF, 0x8010), Instr::Mrs { rd: 0, sysm: 16 });
        // msr basepri, r0 (f380 8811)
        assert_eq!(decode(0xF380, 0x8811), Instr::Msr { rn: 0, sysm: 17, mask: 2 });
        // dsb sy (f3bf 8f4f)
        assert_eq!(decode(0xF3BF, 0x8F4F), Instr::Barrier(Barrier::Dsb));
        // tbb [pc, r0] (e8df f000)
        assert_eq!(decode(0xE8DF, 0xF000), Instr::Tb { half: false, rn: 15, rm: 0 });
        // ubfx r0, r1, #4, #8 (f3c1 1007)
        assert_eq!(decode(0xF3C1, 0x1007), Instr::Bfx { signed: false, rd: 0, rn: 1, lsb: 4, widthm1: 7 });
        // vmov.f32 s0, #1.0 (eeb7 0a00)
        assert_eq!(decode(0xEEB7, 0x0A00), Instr::Vfp(Vfp::MovImm { sd: 0, imm: 0x3F80_0000 }));
        // vcvt.s32.f32 s0, s0 (eebd 0ac0)
        assert_eq!(decode(0xEEBD, 0x0AC0), Instr::Vfp(Vfp::CvtToInt { signed: true, round_zero: true, sd: 0, sm: 0 }));
        // b.w backwards (f7ff bffe) = b.w -4
        assert_eq!(decode(0xF7FF, 0xBFFE), Instr::B { cond: 14, imm: -4 });
        // beq.w +0 (f000 8000)
        assert_eq!(decode(0xF000, 0x8000), Instr::B { cond: 0, imm: 0 });
    }
}

#[cfg(test)]
mod size_tests {
    #[test]
    fn instr_size() {
        eprintln!("size_of::<Instr>() = {}", std::mem::size_of::<super::Instr>());
        assert!(std::mem::size_of::<super::Instr>() <= 24);
    }
}
