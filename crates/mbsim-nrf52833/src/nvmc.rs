//! NVMC: flash write and erase control.
//!
//! The CPU stalls while a write or erase is in progress (code runs from flash), which the model
//! represents with a stall time returned to the core.

use crate::mem::{Memory, FLASH_SIZE, UICR_BASE};
use mbsim_core::time::{ms, us};
use mbsim_core::Ticks;

pub const WRITE_TIME: Ticks = us(41);
pub const PAGE_ERASE_TIME: Ticks = ms(85);
pub const ERASE_ALL_TIME: Ticks = ms(173);

#[derive(Debug, Default)]
pub struct Nvmc {
    pub config: u32,
    pub icachecnf: u32,
    pub ihit: u32,
    pub imiss: u32,
    pub partial_cfg: u32,
}

impl Nvmc {
    pub fn new() -> Self {
        Nvmc { partial_cfg: 0xA, ..Default::default() }
    }

    pub fn read(&mut self, off: u32) -> u32 {
        match off {
            0x400 => 1, // READY
            0x408 => 1, // READYNEXT
            0x504 => self.config,
            0x51C => self.partial_cfg,
            0x540 => self.icachecnf,
            0x548 => self.ihit,
            0x54C => self.imiss,
            _ => 0,
        }
    }

    /// Register write; returns a CPU stall time for erase operations.
    pub fn write(&mut self, off: u32, v: u32, mem: &mut Memory) -> Option<Ticks> {
        let erase_enabled = self.config & 3 == 2;
        match off {
            0x504 => self.config = v & 7,
            0x508 | 0x518
                if erase_enabled
                // ERASEPAGE / ERASEPAGEPARTIAL (modelled as a full page erase)
                && v < FLASH_SIZE =>
            {
                mem.erase_page(v);
                return Some(PAGE_ERASE_TIME);
            }
            0x50C if erase_enabled && v & 1 != 0 => {
                mem.erase_all();
                return Some(ERASE_ALL_TIME);
            }
            0x514 if erase_enabled && v & 1 != 0 => {
                mem.uicr.fill(0xFFFF_FFFF);
                mem.flash_dirty = true;
                return Some(PAGE_ERASE_TIME);
            }
            0x51C => self.partial_cfg = v & 0x7F,
            0x540 => self.icachecnf = v & 0x101,
            0x548 => self.ihit = v,
            0x54C => self.imiss = v,
            _ => {}
        }
        None
    }

    /// CPU write into flash or UICR; returns the stall time when it programs, `Err` for a bus fault.
    #[allow(clippy::result_unit_err)]
    pub fn flash_write(&mut self, addr: u32, size: u8, v: u32, mem: &mut Memory) -> Result<Option<Ticks>, ()> {
        if self.config & 3 != 1 {
            // writes without WEN are ignored by the flash
            return Ok(None);
        }
        if size != 4 || addr & 3 != 0 {
            return Err(());
        }
        if addr < FLASH_SIZE || (UICR_BASE..UICR_BASE + 0x1000).contains(&addr) {
            mem.program_word(addr, v);
            return Ok(Some(WRITE_TIME));
        }
        Ok(None)
    }
}
