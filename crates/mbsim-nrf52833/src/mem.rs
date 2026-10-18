//! Flash, RAM, FICR and UICR.

use crate::env::{RAM_BASE, RAM_SIZE};

pub const FLASH_SIZE: u32 = 0x8_0000;
pub const PAGE_SIZE: u32 = 0x1000;
// the decode cache of the core counts flash writes per flash page
const _: () = assert!(PAGE_SIZE == mbsim_cpu::bus::CODE_PAGE_SIZE);
pub const CODE_RAM_BASE: u32 = 0x0080_0000;
pub const FICR_BASE: u32 = 0x1000_0000;
pub const UICR_BASE: u32 = 0x1000_1000;

pub struct Memory {
    pub flash: Vec<u8>,
    pub ram: Vec<u8>,
    pub ficr: Vec<u32>,
    pub uicr: Vec<u32>,
    /// Set when flash or UICR content changed (for persistence).
    pub flash_dirty: bool,
    /// Incremented on every flash change (invalidates decoded code).
    pub code_gen: u64,
    /// Per flash page: incremented when that page changes (invalidates its decoded code).
    pub page_gen: Vec<u64>,
}

/// FICR words that are not 0xFFFFFFFF on an nRF52833 (QIAA, build code AAB0).
/// To be replaced by a dump from a real board; DEVICEID/DEVICEADDR are placeholders.
const FICR_DEFAULTS: &[(u32, u32)] = &[
    (0x010, 0x0000_1000), // CODEPAGESIZE
    (0x014, 0x0000_0080), // CODESIZE (pages)
    (0x060, 0x1234_5678), // DEVICEID[0]
    (0x064, 0x9ABC_DEF0), // DEVICEID[1]
    (0x0A0, 0xFFFF_FFFF), // DEVICEADDRTYPE (random)
    (0x0A4, 0x5678_9ABC), // DEVICEADDR[0]
    (0x0A8, 0xFFFF_C0DE), // DEVICEADDR[1]
    (0x100, 0x0005_2833), // INFO.PART
    (0x104, 0x4141_4230), // INFO.VARIANT "AAB0"
    (0x108, 0x0000_2004), // INFO.PACKAGE QI
    (0x10C, 0x0000_0080), // INFO.RAM (KB)
    (0x110, 0x0000_0200), // INFO.FLASH (KB)
    (0x130, 0x0000_000D), // part code for the errata checks
    (0x134, 0x0000_0002), // revision code (rev 2: hardware APPROTECT)
    (0x244, 0x0000_0000),
    (0x258, 0x0000_004F),
    (0x324, 0x0000_0000),
    (0x404, 0x0000_0326), // TEMP.A0..A5
    (0x408, 0x0000_0348),
    (0x40C, 0x0000_03AA),
    (0x410, 0x0000_040E),
    (0x414, 0x0000_04BD),
    (0x418, 0x0000_05A3),
    (0x41C, 0x0000_3FCC), // TEMP.B0..B5
    (0x420, 0x0000_3F98),
    (0x424, 0x0000_3F98),
    (0x428, 0x0000_0012),
    (0x42C, 0x0000_004D),
    (0x430, 0x0000_3E10),
    (0x434, 0x0000_00E2), // TEMP.T0..T4
    (0x438, 0x0000_0019),
    (0x43C, 0x0000_003C),
    (0x440, 0x0000_0050),
    (0x444, 0x0000_006E),
];

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Memory {
    pub fn new() -> Self {
        let mut ficr = vec![0xFFFF_FFFF; 1024];
        for &(off, v) in FICR_DEFAULTS {
            ficr[(off / 4) as usize] = v;
        }
        Memory {
            flash: vec![0xFF; FLASH_SIZE as usize],
            ram: vec![0; RAM_SIZE as usize],
            ficr,
            uicr: vec![0xFFFF_FFFF; 1024],
            flash_dirty: false,
            code_gen: 0,
            page_gen: vec![0; (FLASH_SIZE / PAGE_SIZE) as usize],
        }
    }

    pub fn ram_offset(addr: u32) -> Option<usize> {
        if (RAM_BASE..RAM_BASE + RAM_SIZE).contains(&addr) {
            Some((addr - RAM_BASE) as usize)
        } else if (CODE_RAM_BASE..CODE_RAM_BASE + RAM_SIZE).contains(&addr) {
            Some((addr - CODE_RAM_BASE) as usize)
        } else {
            None
        }
    }

    pub fn erase_page(&mut self, addr: u32) {
        let base = (addr & !(PAGE_SIZE - 1)) as usize;
        if base < FLASH_SIZE as usize {
            self.flash[base..base + PAGE_SIZE as usize].fill(0xFF);
            self.flash_changed(addr);
        }
    }

    pub fn erase_all(&mut self) {
        self.flash.fill(0xFF);
        self.uicr.fill(0xFFFF_FFFF);
        self.flash_dirty = true;
        self.code_gen += 1;
        self.page_gen.iter_mut().for_each(|g| *g += 1);
    }

    /// Notes a change of the flash page holding `addr`.
    fn flash_changed(&mut self, addr: u32) {
        self.flash_dirty = true;
        self.code_gen += 1;
        self.page_gen[(addr / PAGE_SIZE) as usize] += 1;
    }

    /// Programs a flash word (bits can only change from 1 to 0).
    pub fn program_word(&mut self, addr: u32, v: u32) {
        let a = (addr & !3) as usize;
        if a + 4 <= FLASH_SIZE as usize {
            let old = u32::from_le_bytes(self.flash[a..a + 4].try_into().unwrap());
            self.flash[a..a + 4].copy_from_slice(&(old & v).to_le_bytes());
            self.flash_changed(addr);
        } else if (UICR_BASE..UICR_BASE + 0x1000).contains(&addr) {
            let i = ((addr - UICR_BASE) / 4) as usize;
            self.uicr[i] &= v;
            self.flash_dirty = true;
        }
    }

    /// Writes an image byte into flash or UICR directly (loader and debugger; assumes erased).
    pub fn load_byte(&mut self, addr: u32, b: u8) -> bool {
        if addr < FLASH_SIZE {
            if self.flash[addr as usize] != b {
                self.flash[addr as usize] = b;
                self.flash_changed(addr);
            }
            true
        } else if (UICR_BASE..UICR_BASE + 0x1000).contains(&addr) {
            let i = ((addr - UICR_BASE) / 4) as usize;
            let sh = (addr & 3) * 8;
            self.uicr[i] = (self.uicr[i] & !(0xFF << sh)) | ((b as u32) << sh);
            self.flash_dirty = true;
            true
        } else if let Some(o) = Self::ram_offset(addr) {
            self.ram[o] = b;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed_pages(m: &Memory, before: &[u64]) -> Vec<usize> {
        m.page_gen.iter().zip(before).enumerate().filter(|(_, (a, b))| a != b).map(|(p, _)| p).collect()
    }

    #[test]
    fn flash_changes_count_per_page() {
        let mut m = Memory::new();
        let g = m.page_gen.clone();
        m.program_word(0x3004, 0x1234_5678);
        m.erase_page(0x5ABC);
        m.load_byte(0x7000, 0xFF); // already erased: no change
        m.load_byte(0x7001, 0x12);
        assert_eq!(changed_pages(&m, &g), [3, 5, 7]);
        assert_eq!(m.code_gen, 3);
        let g = m.page_gen.clone();
        m.erase_all();
        assert_eq!(changed_pages(&m, &g).len(), (FLASH_SIZE / PAGE_SIZE) as usize);
    }
}
