//! EGU0..EGU5 (event generator units, also the software interrupts SWI0..SWI5).

#[derive(Debug, Default)]
pub struct Egu {
    pub events: u32,
    pub pulses: u32,
    pub inten: u32,
}

impl Egu {
    pub fn irq(&self) -> bool {
        self.events & self.inten != 0
    }

    pub fn read(&self, off: u32) -> u32 {
        match off {
            0x100..=0x13C => (self.events >> ((off - 0x100) / 4)) & 1,
            0x300 | 0x304 | 0x308 => self.inten,
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32) {
        match off {
            0x000..=0x03C if v & 1 != 0 => {
                let b = 1 << (off / 4);
                self.events |= b;
                self.pulses |= b;
            }
            0x100..=0x13C => {
                let b = 1 << ((off - 0x100) / 4);
                if v & 1 == 0 {
                    self.events &= !b;
                } else {
                    self.events |= b;
                }
            }
            0x300 => self.inten = v & 0xFFFF,
            0x304 => self.inten |= v & 0xFFFF,
            0x308 => self.inten &= !v,
            _ => {}
        }
    }
}
