//! ARMv7E-M instruction-set simulator of the Cortex-M4F core of the nRF52833.
//!
//! The core executes Thumb-2 including the DSP extension and FPv4-SP, implements the
//! exception model, the System Control Space (NVIC, SCB, SysTick, MPU, FPU control) and the
//! debug components (DCB, DWT, FPB, ITM). Memory and peripherals are reached through [`Bus`].

pub mod alu;
pub mod bus;
pub mod cpu;
pub mod decode;
mod exec;
pub mod fpu;
pub mod scs;

pub use bus::{Bus, BusError, ResetRequest};
pub use cpu::{Cpu, Exit, HaltReason, Watch};
