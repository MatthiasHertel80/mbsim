//! GPIOTE: 8 channels for pin events (IN) and pin tasks (OUT/SET/CLR), and the PORT event from
//! the DETECT signal of GPIO SENSE.

use crate::board::BoardIo;
use crate::gpio::{Gpio, PinOverride};
use mbsim_core::Ticks;

#[derive(Debug, Default)]
pub struct Gpiote {
    /// IN[0..7] in bits 0..7, PORT in bit 31 (INTEN positions).
    pub events: u32,
    pub pulses: u32,
    pub inten: u32,
    pub config: [u32; 8],
    last_level: [bool; 8],
    out_level: [bool; 8],
    detect: bool,
}

impl Gpiote {
    pub fn irq(&self) -> bool {
        self.events & self.inten != 0
    }

    fn pin(c: u32) -> u8 {
        ((c >> 8) & 0x3F) as u8
    }

    fn mode(c: u32) -> u32 {
        c & 3
    }

    pub fn read(&self, off: u32) -> u32 {
        match off {
            0x100..=0x11C => (self.events >> ((off - 0x100) / 4)) & 1,
            0x17C => (self.events >> 31) & 1,
            0x304 | 0x308 => self.inten,
            0x510..=0x52C => self.config[((off - 0x510) / 4) as usize],
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32, now: Ticks, gpio: &mut Gpio, board: &mut dyn BoardIo) {
        match off {
            0x000..=0x01C | 0x030..=0x04C | 0x060..=0x07C if v & 1 != 0 => {
                let n = ((off % 0x30) / 4) as usize;
                let c = self.config[n];
                if Self::mode(c) == 3 {
                    let pol = (c >> 16) & 3;
                    let level = match off / 0x30 {
                        0 => match pol {
                            1 => true,
                            2 => false,
                            3 => !self.out_level[n],
                            _ => self.out_level[n],
                        },
                        1 => true,
                        _ => false,
                    };
                    self.out_level[n] = level;
                    gpio.set_override(Self::pin(c), Some(PinOverride::output(level)), now, board);
                }
            }
            0x100..=0x11C => {
                let b = 1 << ((off - 0x100) / 4);
                if v & 1 == 0 {
                    self.events &= !b;
                }
            }
            0x17C if v & 1 == 0 => {
                self.events &= !(1 << 31);
            }
            0x304 => self.inten |= v & 0x8000_00FF,
            0x308 => self.inten &= !v,
            0x510..=0x52C => {
                let n = ((off - 0x510) / 4) as usize;
                let old = self.config[n];
                if Self::mode(old) != 0 && (Self::mode(v) != Self::mode(old) || Self::pin(v) != Self::pin(old)) {
                    gpio.set_override(Self::pin(old), None, now, board);
                }
                self.config[n] = v & 0x0013_3F03;
                if Self::mode(v) == 3 {
                    self.out_level[n] = (v >> 20) & 1 != 0;
                    gpio.set_override(Self::pin(v), Some(PinOverride::output(self.out_level[n])), now, board);
                } else if Self::mode(v) == 1 {
                    // event mode: the pin is an input
                    gpio.set_override(Self::pin(v), Some(PinOverride::input()), now, board);
                    let d = gpio.drive(Self::pin(v));
                    self.last_level[n] = board.pin_input(now, Self::pin(v), PinDriveInput::connected(d));
                }
            }
            _ => {}
        }
    }

    /// Re-evaluates the inputs of event-mode channels and the DETECT signal.
    pub fn update(&mut self, now: Ticks, gpio: &mut Gpio, board: &mut dyn BoardIo) {
        for n in 0..8 {
            let c = self.config[n];
            if Self::mode(c) != 1 {
                continue;
            }
            let d = gpio.drive(Self::pin(c));
            let level = board.pin_input(now, Self::pin(c), PinDriveInput::connected(d));
            let was = self.last_level[n];
            if level != was {
                self.last_level[n] = level;
                let hit = match (c >> 16) & 3 {
                    1 => level,
                    2 => !level,
                    3 => true,
                    _ => false,
                };
                if hit {
                    self.events |= 1 << n;
                    self.pulses |= 1 << n;
                }
            }
        }
        let detect = gpio.detect(now, board);
        if detect && !self.detect {
            self.events |= 1 << 31;
            self.pulses |= 1 << 31;
        }
        self.detect = detect;
    }
}

/// GPIOTE event mode reads the pin through a connected input buffer.
struct PinDriveInput;

impl PinDriveInput {
    fn connected(mut d: crate::board::PinDrive) -> crate::board::PinDrive {
        d.input = true;
        d
    }
}
