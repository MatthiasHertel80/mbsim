//! The interface MCU (nRF52820 with DAPLink) as seen by the target: the I2C protocol of
//! tech.microbit.org/software/spec-i2c-protocol (config interface at 0x70, flash storage at
//! 0x72) and the shared COMBINED_SENSOR_INT line (P0.25, active low) it pulls while a response
//! is ready.

pub const CONFIG_ADDR: u8 = 0x70;
pub const STORAGE_ADDR: u8 = 0x72;

const NOP: u8 = 0x00;
const READ_REQUEST: u8 = 0x10;
const READ_RESPONSE: u8 = 0x11;
const WRITE_REQUEST: u8 = 0x12;
const WRITE_RESPONSE: u8 = 0x13;
const ERROR_RESPONSE: u8 = 0x20;

const ERR_INCOMPLETE: u8 = 0x31;
const ERR_UNKNOWN_COMMAND: u8 = 0x32;
const ERR_UNKNOWN_PROPERTY: u8 = 0x34;
const ERR_WRONG_SIZE: u8 = 0x35;
const ERR_READ_DISALLOWED: u8 = 0x36;
const ERR_WRITE_DISALLOWED: u8 = 0x37;
const ERR_BUSY: u8 = 0x39;

/// Storage of the flash interface (128 KB of the interface MCU's flash).
const STORAGE_SIZE: usize = 128 * 1024;
const SECTOR: usize = 1024;

pub struct InterfaceMcu {
    /// Response waiting to be read (the interrupt line is asserted while it waits).
    response: Option<Vec<u8>>,
    /// DAPLink board ID (0x9904: micro:bit V2.2), DAPLink version 0257.
    pub board_id: u16,
    pub daplink_version: u16,
    /// Power state: 1 = USB only.
    pub power_state: u8,
    /// USB enumeration state: 4 = configured.
    pub usb_state: u8,
    pub power_led_on: bool,
    pub auto_sleep: bool,
    /// The target asked the interface to power down (System OFF follows).
    pub power_down_requested: bool,
    pub storage: Vec<u8>,
    file_name: [u8; 11],
    file_size: u32,
    visible: bool,
    enc_window: (u32, u32),
}

impl Default for InterfaceMcu {
    fn default() -> Self {
        Self::new()
    }
}

impl InterfaceMcu {
    pub fn new() -> Self {
        InterfaceMcu {
            response: None,
            board_id: 0x9904,
            daplink_version: 257,
            power_state: 1,
            usb_state: 4,
            power_led_on: true,
            auto_sleep: false,
            power_down_requested: false,
            storage: vec![0xFF; STORAGE_SIZE],
            file_name: *b"DATA    BIN",
            file_size: 129_024,
            visible: false,
            enc_window: (0, 0),
        }
    }

    pub fn reset(&mut self) {
        self.response = None;
    }

    /// The interrupt line is pulled while a response waits.
    pub fn irq(&self) -> bool {
        self.response.is_some()
    }

    fn error(code: u8) -> Vec<u8> {
        vec![ERROR_RESPONSE, code]
    }

    /// I2C write to the config interface (0x70).
    pub fn config_write(&mut self, data: &[u8]) {
        let Some(&cmd) = data.first() else { return };
        if self.response.is_some() && cmd != NOP {
            // the previous response was not read; the interface answers busy
            self.response = Some(Self::error(ERR_BUSY));
            return;
        }
        let r = match cmd {
            NOP => return,
            READ_REQUEST => match data.get(1) {
                None => Self::error(ERR_INCOMPLETE),
                Some(&p) => {
                    let d: Option<Vec<u8>> = match p {
                        0x01 => Some(self.board_id.to_le_bytes().to_vec()),
                        0x02 => Some(1u16.to_le_bytes().to_vec()),
                        0x03 => Some(self.daplink_version.to_le_bytes().to_vec()),
                        0x04 => Some(vec![self.power_state]),
                        0x05 => {
                            // bat_sense and VIN in microvolts
                            let mut v = 0u32.to_le_bytes().to_vec();
                            v.extend(3_300_000u32.to_le_bytes());
                            Some(v)
                        }
                        0x06 => Some(vec![self.usb_state]),
                        0x07 | 0x08 | 0x0A => None,
                        _ => {
                            self.response = Some(Self::error(ERR_UNKNOWN_PROPERTY));
                            return;
                        }
                    };
                    match d {
                        Some(d) => {
                            let mut v = vec![READ_RESPONSE, p, d.len() as u8];
                            v.extend(d);
                            v
                        }
                        None => Self::error(ERR_READ_DISALLOWED),
                    }
                }
            },
            WRITE_REQUEST => {
                if data.len() < 3 {
                    Self::error(ERR_INCOMPLETE)
                } else {
                    let (p, n) = (data[1], data[2] as usize);
                    let payload = &data[3..];
                    if payload.len() < n {
                        Self::error(ERR_INCOMPLETE)
                    } else {
                        match p {
                            0x07 | 0x08 | 0x0A if n != 1 => Self::error(ERR_WRONG_SIZE),
                            0x07 => {
                                self.power_down_requested = payload[0] == 0x08;
                                vec![WRITE_RESPONSE, p]
                            }
                            0x08 => {
                                self.power_led_on = payload[0] != 0;
                                vec![WRITE_RESPONSE, p]
                            }
                            0x0A => {
                                self.auto_sleep = payload[0] != 0;
                                vec![WRITE_RESPONSE, p]
                            }
                            0x01..=0x06 | 0x09 => Self::error(ERR_WRITE_DISALLOWED),
                            _ => Self::error(ERR_UNKNOWN_PROPERTY),
                        }
                    }
                }
            }
            _ => Self::error(ERR_UNKNOWN_COMMAND),
        };
        self.response = Some(r);
    }

    /// I2C read from the config interface; releases the interrupt line.
    pub fn config_read(&mut self, buf: &mut [u8]) {
        let r = self.response.take().unwrap_or_else(|| Self::error(ERR_BUSY));
        for (i, b) in buf.iter_mut().enumerate() {
            *b = r.get(i).copied().unwrap_or(0);
        }
    }

    /// Reset button long press: user event 0x03 (asynchronous read response).
    pub fn user_event(&mut self, ev: u8) {
        if self.response.is_none() {
            self.response = Some(vec![READ_RESPONSE, 0x09, 1, ev]);
        }
    }

    fn be24(b: &[u8]) -> usize {
        ((b[0] as usize) << 16) | ((b[1] as usize) << 8) | b[2] as usize
    }

    fn be32(b: &[u8]) -> usize {
        ((b[0] as usize) << 24) | ((b[1] as usize) << 16) | ((b[2] as usize) << 8) | b[3] as usize
    }

    /// I2C write to the flash storage interface (0x72).
    pub fn storage_write(&mut self, data: &[u8]) {
        let Some(&cmd) = data.first() else { return };
        let p = &data[1..];
        let r = match cmd {
            0x01 if p.len() >= 11 => {
                self.file_name.copy_from_slice(&p[..11]);
                let mut v = vec![cmd];
                v.extend(self.file_name);
                v
            }
            0x02 if p.len() >= 4 => {
                self.file_size = Self::be32(p) as u32;
                let mut v = vec![cmd];
                v.extend(self.file_size.to_be_bytes());
                v
            }
            0x03 if !p.is_empty() => {
                self.visible = p[0] != 0;
                vec![cmd, self.visible as u8]
            }
            0x09 if p.len() >= 8 => {
                self.enc_window = (Self::be32(p) as u32, Self::be32(&p[4..]) as u32);
                let mut v = vec![cmd];
                v.extend(self.enc_window.0.to_be_bytes());
                v.extend(self.enc_window.1.to_be_bytes());
                v
            }
            0x04 | 0x08 => vec![cmd],
            0x05 => {
                self.file_name = *b"DATA    BIN";
                self.file_size = 129_024;
                self.visible = false;
                self.enc_window = (0, 0);
                vec![cmd]
            }
            0x06 => vec![cmd, (STORAGE_SIZE / 1024) as u8],
            0x07 => {
                let mut v = vec![cmd];
                v.extend((SECTOR as u16).to_be_bytes());
                v
            }
            0x0A if p.len() >= 7 => {
                let (addr, len) = (Self::be24(p), Self::be32(&p[3..]));
                let mut v = vec![cmd];
                v.extend_from_slice(&p[..7]);
                for i in 0..len.min(1024) {
                    v.push(self.storage.get(addr + i).copied().unwrap_or(0xFF));
                }
                v
            }
            0x0B if p.len() >= 7 => {
                let (addr, len) = (Self::be24(p), Self::be32(&p[3..]));
                let payload = &p[7..];
                if addr % 4 != 0 || len % 4 != 0 || addr + len > STORAGE_SIZE || payload.len() < len {
                    Self::error(0x38)
                } else {
                    for (i, b) in payload[..len].iter().enumerate() {
                        self.storage[addr + i] &= *b;
                    }
                    let mut v = vec![cmd];
                    v.extend_from_slice(&p[..7]);
                    v
                }
            }
            0x0C if p.len() >= 7 => {
                let (start, end) = (Self::be24(p), Self::be24(&p[4..]));
                if start % SECTOR != 0 || end < start || end >= STORAGE_SIZE {
                    Self::error(0x38)
                } else {
                    let end_sector = (end / SECTOR + 1) * SECTOR;
                    self.storage[start..end_sector.min(STORAGE_SIZE)].fill(0xFF);
                    let mut v = vec![cmd];
                    v.extend_from_slice(&p[..7]);
                    v
                }
            }
            _ => Self::error(ERR_UNKNOWN_COMMAND),
        };
        self.response = Some(r);
    }

    pub fn storage_read(&mut self, buf: &mut [u8]) {
        self.config_read(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_board_version() {
        let mut i = InterfaceMcu::new();
        i.config_write(&[0x10, 0x01]);
        assert!(i.irq());
        let mut b = [0u8; 5];
        i.config_read(&mut b);
        assert_eq!(b, [0x11, 0x01, 0x02, 0x04, 0x99]);
        assert!(!i.irq());
    }
}
