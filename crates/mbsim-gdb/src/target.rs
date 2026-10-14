//! Target description of the Cortex-M4F for GDB: registers, target.xml, memory map.

use mbsim_cpu::Cpu;

/// Register numbers of the target description:
/// 0..15 r0..pc, 16 xpsr, 17 msp, 18 psp, 19 primask, 20 basepri, 21 faultmask, 22 control,
/// 23..38 d0..d15, 39 fpscr.
pub const NUM_REGS: u32 = 40;

#[derive(Clone, Copy, Debug)]
pub enum Sel {
    Core(u32),
    Special(u8),
    Double(u32),
}

pub fn reg_map(n: u32) -> Option<Sel> {
    Some(match n {
        0..=18 => Sel::Core(n),
        19..=22 => Sel::Special((n - 19) as u8),
        23..=38 => Sel::Double(n - 23),
        39 => Sel::Core(33),
        _ => return None,
    })
}

pub fn width(s: Sel) -> usize {
    match s {
        Sel::Double(_) => 8,
        _ => 4,
    }
}

pub fn read(cpu: &Cpu, s: Sel) -> Option<Vec<u8>> {
    Some(match s {
        Sel::Core(n) => cpu.read_core_reg(n)?.to_le_bytes().to_vec(),
        Sel::Special(i) => {
            let v = match i {
                0 => cpu.primask as u32,
                1 => cpu.basepri as u32,
                2 => cpu.faultmask as u32,
                _ => cpu.control(),
            };
            v.to_le_bytes().to_vec()
        }
        Sel::Double(d) => {
            let lo = cpu.s[(2 * d) as usize];
            let hi = cpu.s[(2 * d + 1) as usize];
            let mut v = lo.to_le_bytes().to_vec();
            v.extend(hi.to_le_bytes());
            v
        }
    })
}

pub fn write(cpu: &mut Cpu, s: Sel, data: &[u8]) -> bool {
    let w = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    if data.len() < width(s) {
        return false;
    }
    match s {
        Sel::Core(n) => cpu.write_core_reg(n, w(0)),
        Sel::Special(i) => {
            let v = w(0);
            match i {
                0 => cpu.set_primask(v & 1 != 0),
                1 => cpu.set_basepri(v as u8),
                2 => cpu.set_faultmask(v & 1 != 0),
                _ => cpu.set_control(v & 7),
            }
            true
        }
        Sel::Double(d) => {
            cpu.s[(2 * d) as usize] = w(0);
            cpu.s[(2 * d + 1) as usize] = w(4);
            true
        }
    }
}

pub fn target_xml() -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE target SYSTEM \"gdb-target.dtd\">\n<target version=\"1.0\">\n<architecture>arm</architecture>\n<feature name=\"org.gnu.gdb.arm.m-profile\">\n",
    );
    for i in 0..13 {
        s += &format!("<reg name=\"r{i}\" bitsize=\"32\" regnum=\"{i}\" type=\"uint32\" group=\"general\"/>\n");
    }
    s += "<reg name=\"sp\" bitsize=\"32\" regnum=\"13\" type=\"data_ptr\" group=\"general\"/>\n";
    s += "<reg name=\"lr\" bitsize=\"32\" regnum=\"14\" type=\"uint32\" group=\"general\"/>\n";
    s += "<reg name=\"pc\" bitsize=\"32\" regnum=\"15\" type=\"code_ptr\" group=\"general\"/>\n";
    s += "<reg name=\"xpsr\" bitsize=\"32\" regnum=\"16\" type=\"uint32\" group=\"general\"/>\n</feature>\n";
    s += "<feature name=\"org.gnu.gdb.arm.m-system\">\n";
    s += "<reg name=\"msp\" bitsize=\"32\" regnum=\"17\" type=\"data_ptr\" group=\"system\"/>\n";
    s += "<reg name=\"psp\" bitsize=\"32\" regnum=\"18\" type=\"data_ptr\" group=\"system\"/>\n";
    s += "<reg name=\"primask\" bitsize=\"32\" regnum=\"19\" type=\"uint32\" group=\"system\"/>\n";
    s += "<reg name=\"basepri\" bitsize=\"32\" regnum=\"20\" type=\"uint32\" group=\"system\"/>\n";
    s += "<reg name=\"faultmask\" bitsize=\"32\" regnum=\"21\" type=\"uint32\" group=\"system\"/>\n";
    s += "<reg name=\"control\" bitsize=\"32\" regnum=\"22\" type=\"uint32\" group=\"system\"/>\n</feature>\n";
    s += "<feature name=\"org.gnu.gdb.arm.vfp\">\n";
    for i in 0..16 {
        s += &format!("<reg name=\"d{i}\" bitsize=\"64\" regnum=\"{}\" type=\"ieee_double\" group=\"float\"/>\n", 23 + i);
    }
    s += "<reg name=\"fpscr\" bitsize=\"32\" regnum=\"39\" type=\"int\" group=\"float\"/>\n</feature>\n</target>\n";
    s
}

pub fn memory_map_xml() -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE memory-map PUBLIC \"+//IDN gnu.org//DTD GDB Memory Map V1.0//EN\" \"http://sourceware.org/gdb/gdb-memory-map.dtd\">\n<memory-map>\n",
    );
    s += "<memory type=\"flash\" start=\"0x0\" length=\"0x80000\"><property name=\"blocksize\">0x1000</property></memory>\n";
    s += "<memory type=\"ram\" start=\"0x800000\" length=\"0x20000\"/>\n";
    s += "<memory type=\"rom\" start=\"0x10000000\" length=\"0x1000\"/>\n";
    s += "<memory type=\"flash\" start=\"0x10001000\" length=\"0x1000\"><property name=\"blocksize\">0x1000</property></memory>\n";
    s += "<memory type=\"ram\" start=\"0x20000000\" length=\"0x20000\"/>\n";
    s += "<memory type=\"ram\" start=\"0x40000000\" length=\"0x80000\"/>\n";
    s += "<memory type=\"ram\" start=\"0x50000000\" length=\"0x1000\"/>\n";
    s += "<memory type=\"ram\" start=\"0xe0000000\" length=\"0x100000\"/>\n";
    s += "</memory-map>\n";
    s
}

pub fn monitor_help() -> &'static str {
    "Monitor commands of the micro:bit V2 simulator:\n\
     \x20 reset [halt|run]          reset the board (default: halt at the reset vector)\n\
     \x20 halt                      halt the core\n\
     \x20 sim time                  simulated time\n\
     \x20 sim stats                 instructions, MIPS, real-time factor\n\
     \x20 sim speed <x|max|realtime> simulation speed\n\
     \x20 sim button a|b press|release\n\
     \x20 sim touch on|off          touch the logo\n\
     \x20 sim tilt <roll> <pitch>   board orientation in degrees\n\
     \x20 sim heading <deg>         compass heading of the top edge\n\
     \x20 sim temp <celsius>        die and sensor temperature\n\
     \x20 sim sound <mV> [Hz]       sound at the microphone (0 = off)\n\
     \x20 sim clap                  a loud clap at the microphone\n\
     \x20 sim uart <text>           send text to the target UART\n\
     \x20 sim leds                  LED matrix state\n\
     \x20 sim warnings              model warnings\n"
}
