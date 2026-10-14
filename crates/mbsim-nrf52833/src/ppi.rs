//! PPI: programmable peripheral interconnect. 20 programmable channels, 12 fixed channels
//! (20..31), 6 channel groups and a fork task per channel. Events of the peripherals are routed
//! by [`crate::System`] through [`Ppi::route`].

#[derive(Debug)]
pub struct Ppi {
    pub chen: u32,
    pub eep: [u32; 20],
    pub tep: [u32; 20],
    pub fork: [u32; 32],
    pub chg: [u32; 6],
}

/// Pre-programmed channels 20..31 (PS table "Pre-programmed channels").
const FIXED: [(u32, u32); 12] = [
    (0x4000_8140, 0x4000_1000), // TIMER0 COMPARE[0] -> RADIO TXEN
    (0x4000_8140, 0x4000_1004), // TIMER0 COMPARE[0] -> RADIO RXEN
    (0x4000_8144, 0x4000_1010), // TIMER0 COMPARE[1] -> RADIO DISABLE
    (0x4000_1128, 0x4000_F000), // RADIO BCMATCH -> AAR START
    (0x4000_1100, 0x4000_F000), // RADIO READY -> CCM KSGEN
    (0x4000_1104, 0x4000_F004), // RADIO ADDRESS -> CCM CRYPT
    (0x4000_1104, 0x4000_8044), // RADIO ADDRESS -> TIMER0 CAPTURE[1]
    (0x4000_110C, 0x4000_8048), // RADIO END -> TIMER0 CAPTURE[2]
    (0x4000_B140, 0x4000_1000), // RTC0 COMPARE[0] -> RADIO TXEN
    (0x4000_B140, 0x4000_1004), // RTC0 COMPARE[0] -> RADIO RXEN
    (0x4000_B140, 0x4000_800C), // RTC0 COMPARE[0] -> TIMER0 CLEAR
    (0x4000_B140, 0x4000_8000), // RTC0 COMPARE[0] -> TIMER0 START
];

impl Default for Ppi {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppi {
    pub fn new() -> Self {
        Ppi { chen: 0, eep: [0; 20], tep: [0; 20], fork: [0; 32], chg: [0; 6] }
    }

    pub fn active(&self) -> bool {
        self.chen != 0
    }

    pub fn read(&self, off: u32) -> u32 {
        match off {
            0x500..=0x508 => self.chen,
            0x510..=0x5AC => {
                let i = ((off - 0x510) / 8) as usize;
                if (off - 0x510).is_multiple_of(8) {
                    self.eep[i]
                } else {
                    self.tep[i]
                }
            }
            0x800..=0x814 => self.chg[((off - 0x800) / 4) as usize],
            0x910..=0x98C => self.fork[((off - 0x910) / 4) as usize],
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, v: u32) {
        match off {
            0x000..=0x02C
                // TASKS_CHG[n].EN / .DIS
                if v & 1 != 0 => {
                    let n = (off / 8) as usize;
                    if off.is_multiple_of(8) {
                        self.chen |= self.chg[n];
                    } else {
                        self.chen &= !self.chg[n];
                    }
                }
            0x500 => self.chen = v,
            0x504 => self.chen |= v,
            0x508 => self.chen &= !v,
            0x510..=0x5AC => {
                let i = ((off - 0x510) / 8) as usize;
                if (off - 0x510).is_multiple_of(8) {
                    self.eep[i] = v;
                } else {
                    self.tep[i] = v;
                }
            }
            0x800..=0x814 => self.chg[((off - 0x800) / 4) as usize] = v,
            0x910..=0x98C => self.fork[((off - 0x910) / 4) as usize] = v,
            _ => {}
        }
    }

    /// Task addresses triggered by an event at `event_addr`.
    pub fn route(&self, event_addr: u32, out: &mut Vec<u32>) {
        let mut ch = self.chen;
        while ch != 0 {
            let i = ch.trailing_zeros() as usize;
            ch &= ch - 1;
            let (eep, tep) = if i < 20 { (self.eep[i], self.tep[i]) } else { FIXED[i - 20] };
            if eep == event_addr && eep != 0 {
                if tep != 0 {
                    out.push(tep);
                }
                if self.fork[i] != 0 {
                    out.push(self.fork[i]);
                }
            }
        }
    }
}
