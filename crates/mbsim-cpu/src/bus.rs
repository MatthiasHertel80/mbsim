//! The interface between the core and the rest of the system (memories, peripherals).
//!
//! The private peripheral bus (0xE000_0000..0xE00F_FFFF: NVIC, SCB, SysTick, MPU, debug
//! components) is part of the core and never reaches the [`Bus`].

use mbsim_core::Ticks;

/// Page size of the decode cache: the unit of [`Bus::code_page_generation`].
pub const CODE_PAGE_SIZE: u32 = 0x1000;

/// A bus error response; the core turns it into a BusFault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusError;

/// Reason for a reset request from the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetRequest {
    /// `AIRCR.SYSRESETREQ`: system reset of the device.
    System,
    /// The core locked up; the nRF52833 resets the system.
    Lockup,
}

pub trait Bus {
    /// Data read of 1, 2 or 4 bytes at `addr` (aligned to the size by the core, except for
    /// unaligned word/halfword accesses to normal memory). `debug` accesses come from the
    /// debugger and must not have side effects.
    fn read(&mut self, now: Ticks, addr: u32, size: u8, debug: bool) -> Result<u32, BusError>;

    /// Data write of 1, 2 or 4 bytes.
    fn write(&mut self, now: Ticks, addr: u32, size: u8, value: u32, debug: bool) -> Result<(), BusError>;

    /// Instruction fetch of a halfword.
    fn fetch16(&mut self, addr: u32) -> Result<u16, BusError>;

    /// The core must stop before starting an instruction at or after this time, so that the
    /// system can process its events.
    fn deadline(&self) -> Ticks;

    /// Interrupt line levels (bit n = IRQ n) when they changed since the last call.
    fn take_irq_lines(&mut self) -> Option<u64>;

    /// Extra wait cycles of an instruction fetch at `addr` (flash wait states).
    fn fetch_wait_cycles(&self, _addr: u32) -> u32 {
        0
    }

    /// Time until which the core stalls after the last access (flash programming), if any.
    fn take_stall(&mut self) -> Option<Ticks> {
        None
    }

    /// Instructions below this address may be cached in decoded form (read-only code memory).
    fn code_cache_limit(&self) -> u32 {
        0
    }

    /// Changes whenever the cacheable code memory is written (flash programming).
    fn code_generation(&self) -> u64 {
        0
    }

    /// Changes whenever page `page` ([`CODE_PAGE_SIZE`] bytes) of the cacheable code memory is
    /// written. The default follows [`Bus::code_generation`], so any write drops all decoded code.
    fn code_page_generation(&self, _page: u32) -> u64 {
        self.code_generation()
    }
}
