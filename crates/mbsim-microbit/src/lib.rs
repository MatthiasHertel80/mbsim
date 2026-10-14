//! The BBC micro:bit V2 (V2.2) board model and the [`Machine`] that runs it.

pub mod board;
pub mod interface;
pub mod lsm303agr;
pub mod machine;
pub mod pins;
pub mod runtime;
mod semihost;

pub use board::MicrobitBoard;
pub use machine::{Machine, Stop};
pub use runtime::{Simulator, Speed};
