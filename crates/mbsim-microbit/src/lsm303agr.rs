//! ST LSM303AGR accelerometer (I2C 0x19) and magnetometer (I2C 0x1E), register level
//! (datasheet DocID027765 Rev 11).
//!
//! Inputs are the specific force (mg) and the magnetic field (mgauss) in the sensor frame.
//! Outputs follow the configured mode: power-down, low-power (8 bit), normal (10 bit) and
//! high-resolution (12 bit), left-justified two's complement; ODR determines when new data
//! arrive (STATUS bits); BDU and BLE are honoured.

use mbsim_core::time::TICKS_PER_SEC;
use mbsim_core::Ticks;

pub const ACC_ADDR: u8 = 0x19;
pub const MAG_ADDR: u8 = 0x1E;

#[derive(Clone, Debug)]
pub struct Lsm303agr {
    pub acc_regs: [u8; 0x40],
    pub mag_regs: [u8; 0x70],
    acc_ptr: u8,
    acc_autoinc: bool,
    mag_ptr: u8,
    /// Specific force in mg (sensor frame).
    pub accel_mg: [f64; 3],
    /// Magnetic field in mgauss (sensor frame).
    pub mag_mgauss: [f64; 3],
    /// Temperature in degrees Celsius.
    pub temperature: f64,
    acc_last_read: u64,
    mag_last_read: u64,
    mag_single_until: Option<Ticks>,
    /// Output latched for BDU between the low and high byte reads.
    acc_latched: Option<[u8; 6]>,
    mag_latched: Option<[u8; 6]>,
}

impl Default for Lsm303agr {
    fn default() -> Self {
        Self::new()
    }
}

impl Lsm303agr {
    pub fn new() -> Self {
        let mut s = Lsm303agr {
            acc_regs: [0; 0x40],
            mag_regs: [0; 0x70],
            acc_ptr: 0,
            acc_autoinc: false,
            mag_ptr: 0,
            accel_mg: [0.0, 0.0, -1000.0],
            mag_mgauss: [0.0; 3],
            temperature: 25.0,
            acc_last_read: 0,
            mag_last_read: 0,
            mag_single_until: None,
            acc_latched: None,
            mag_latched: None,
        };
        s.reset();
        s
    }

    pub fn reset(&mut self) {
        self.acc_regs = [0; 0x40];
        self.mag_regs = [0; 0x70];
        self.acc_regs[0x0F] = 0x33;
        self.acc_regs[0x20] = 0x07;
        self.mag_regs[0x4F] = 0x40;
        self.mag_regs[0x60] = 0x03;
        self.acc_latched = None;
        self.mag_latched = None;
        self.mag_single_until = None;
    }

    fn acc_odr(&self) -> f64 {
        match self.acc_regs[0x20] >> 4 {
            0 => 0.0,
            1 => 1.0,
            2 => 10.0,
            3 => 25.0,
            4 => 50.0,
            5 => 100.0,
            6 => 200.0,
            7 => 400.0,
            8 => 1620.0,
            9 => {
                if self.acc_regs[0x20] & 8 != 0 {
                    5376.0
                } else {
                    1344.0
                }
            }
            _ => 0.0,
        }
    }

    fn sample_index(now: Ticks, odr: f64) -> u64 {
        if odr <= 0.0 {
            0
        } else {
            (now as f64 / TICKS_PER_SEC as f64 * odr) as u64
        }
    }

    /// Accelerometer output bytes OUT_X_L..OUT_Z_H for the current mode.
    fn acc_output(&self) -> [u8; 6] {
        let ctrl1 = self.acc_regs[0x20];
        let ctrl4 = self.acc_regs[0x23];
        if ctrl1 >> 4 == 0 {
            return [0; 6];
        }
        let lp = ctrl1 & 8 != 0;
        let hr = ctrl4 & 8 != 0 && !lp;
        let fs = (ctrl4 >> 4) & 3;
        // sensitivity in mg/digit (datasheet table 3)
        let (bits, sens) = match (lp, hr) {
            (true, _) => (8, [16.0, 32.0, 64.0, 192.0][fs as usize]),
            (false, true) => (12, [0.98, 1.95, 3.9, 11.72][fs as usize]),
            (false, false) => (10, [3.9, 7.82, 15.63, 46.9][fs as usize]),
        };
        let max = (1i32 << (bits - 1)) - 1;
        let mut out = [0u8; 6];
        for i in 0..3 {
            let enabled = ctrl1 & (1 << i) != 0;
            let v = if enabled { (self.accel_mg[i] / sens).round().clamp(-(max as f64) - 1.0, max as f64) as i32 } else { 0 };
            let raw = ((v << (16 - bits)) as i16).to_le_bytes();
            let (lo, hi) = if ctrl4 & 0x40 != 0 { (raw[1], raw[0]) } else { (raw[0], raw[1]) };
            out[2 * i] = lo;
            out[2 * i + 1] = hi;
        }
        out
    }

    fn mag_output(&self, now: Ticks) -> [u8; 6] {
        let cfg_a = self.mag_regs[0x60];
        let md = cfg_a & 3;
        let active = md == 0 || self.mag_single_until.is_some_and(|t| now >= t);
        if !active && self.mag_latched.is_none() {
            return [
                self.mag_regs[0x68],
                self.mag_regs[0x69],
                self.mag_regs[0x6A],
                self.mag_regs[0x6B],
                self.mag_regs[0x6C],
                self.mag_regs[0x6D],
            ];
        }
        let ble = self.mag_regs[0x62] & 0x02 != 0;
        let mut out = [0u8; 6];
        for i in 0..3 {
            let off = i16::from_le_bytes([self.mag_regs[0x45 + 2 * i], self.mag_regs[0x46 + 2 * i]]) as f64;
            let v = (self.mag_mgauss[i] / 1.5 - off).round().clamp(-32768.0, 32767.0) as i16;
            let raw = v.to_le_bytes();
            let (lo, hi) = if ble { (raw[1], raw[0]) } else { (raw[0], raw[1]) };
            out[2 * i] = lo;
            out[2 * i + 1] = hi;
        }
        out
    }

    fn mag_odr(&self) -> f64 {
        [10.0, 20.0, 50.0, 100.0][((self.mag_regs[0x60] >> 2) & 3) as usize]
    }

    fn acc_read_reg(&mut self, reg: u8, now: Ticks) -> u8 {
        let reg = reg & 0x3F;
        match reg {
            0x27 => {
                // STATUS_REG_A: ZYXDA/ZYXOR from the sample count
                let idx = Self::sample_index(now, self.acc_odr());
                if idx > self.acc_last_read && self.acc_odr() > 0.0 {
                    let over = idx > self.acc_last_read + 1;
                    0x0F | if over { 0xF0 } else { 0 }
                } else {
                    0
                }
            }
            0x28..=0x2D => {
                let bdu = self.acc_regs[0x23] & 0x80 != 0;
                let data = if bdu {
                    if reg == 0x28 || self.acc_latched.is_none() {
                        self.acc_latched = Some(self.acc_output());
                    }
                    self.acc_latched.unwrap()
                } else {
                    self.acc_output()
                };
                let v = data[(reg - 0x28) as usize];
                if reg == 0x2D {
                    self.acc_latched = None;
                    self.acc_last_read = Self::sample_index(now, self.acc_odr());
                }
                v
            }
            0x0C | 0x0D => {
                // OUT_TEMP: 8-bit two's complement, 1 digit/degree, 0 at 25 degrees (left-justified)
                if self.acc_regs[0x1F] & 0xC0 == 0xC0 {
                    let t = (self.temperature - 25.0).round().clamp(-128.0, 127.0) as i8;
                    if reg == 0x0D {
                        t as u8
                    } else {
                        0
                    }
                } else {
                    0
                }
            }
            0x07 => 0x04,
            _ => self.acc_regs[reg as usize],
        }
    }

    fn acc_write_reg(&mut self, reg: u8, v: u8) {
        let reg = reg & 0x3F;
        match reg {
            0x1F..=0x26 | 0x2E | 0x30 | 0x32..=0x34 | 0x36 | 0x37 | 0x38 | 0x3A..=0x3F => self.acc_regs[reg as usize] = v,
            _ => {}
        }
        if reg == 0x24 && v & 0x80 != 0 {
            // BOOT: reload trimming, registers keep their values
            self.acc_regs[0x24] &= !0x80;
        }
    }

    fn mag_read_reg(&mut self, reg: u8, now: Ticks) -> u8 {
        match reg {
            0x67 => {
                let idx = Self::sample_index(now, self.mag_odr());
                let md = self.mag_regs[0x60] & 3;
                let ready = (md == 0 && idx > self.mag_last_read) || self.mag_single_until.is_some_and(|t| now >= t);
                if ready {
                    0x0F
                } else {
                    0
                }
            }
            0x68..=0x6D => {
                let bdu = self.mag_regs[0x62] & 0x10 != 0;
                let data = if bdu {
                    if reg == 0x68 || self.mag_latched.is_none() {
                        self.mag_latched = Some(self.mag_output(now));
                    }
                    self.mag_latched.unwrap()
                } else {
                    self.mag_output(now)
                };
                let v = data[(reg - 0x68) as usize];
                if reg == 0x6D {
                    self.mag_latched = None;
                    self.mag_last_read = Self::sample_index(now, self.mag_odr());
                    if let Some(t) = self.mag_single_until {
                        if now >= t {
                            // single measurement done: keep the data, back to idle
                            self.mag_regs[0x68..0x6E].copy_from_slice(&data);
                            self.mag_regs[0x60] = (self.mag_regs[0x60] & !3) | 3;
                            self.mag_single_until = None;
                        }
                    }
                }
                v
            }
            0x45..=0x4A | 0x4F | 0x60..=0x66 => self.mag_regs[reg as usize],
            _ => 0,
        }
    }

    fn mag_write_reg(&mut self, reg: u8, v: u8, now: Ticks) {
        match reg {
            0x45..=0x4A | 0x61..=0x63 | 0x65 | 0x66 => self.mag_regs[reg as usize] = v,
            0x60 => {
                if v & 0x20 != 0 {
                    // SOFT_RST
                    self.mag_regs[0x60..0x67].fill(0);
                    self.mag_regs[0x60] = 0x03;
                    return;
                }
                if v & 0x40 != 0 {
                    // REBOOT: memory content reloaded
                }
                self.mag_regs[0x60] = v & 0x9F;
                if v & 3 == 1 {
                    // single measurement: data after one ODR period
                    let t = now + (TICKS_PER_SEC as f64 / self.mag_odr().max(10.0)) as Ticks;
                    self.mag_single_until = Some(t);
                }
            }
            _ => {}
        }
    }

    /// INT1 (accelerometer data ready, CTRL_REG3_A I1_ZYXDA) is active.
    pub fn int1_active(&self, now: Ticks) -> bool {
        self.acc_regs[0x22] & 0x10 != 0 && self.acc_odr() > 0.0 && Self::sample_index(now, self.acc_odr()) > self.acc_last_read
    }

    /// The INT_MAG/DRDY pin with DRDY_on_PIN (CFG_REG_C_M bit 0) is active.
    pub fn drdy_active(&self, now: Ticks) -> bool {
        let md = self.mag_regs[0x60] & 3;
        self.mag_regs[0x62] & 0x01 != 0
            && ((md == 0 && Self::sample_index(now, self.mag_odr()) > self.mag_last_read)
                || self.mag_single_until.is_some_and(|t| now >= t))
    }

    /// Next time an interrupt output becomes active (for the board's event scheduling).
    pub fn next_interrupt(&self, now: Ticks) -> Ticks {
        let mut t = Ticks::MAX;
        let next = |odr: f64, idx: u64| ((idx + 1) as f64 / odr * TICKS_PER_SEC as f64).ceil() as Ticks;
        if self.acc_regs[0x22] & 0x10 != 0 && self.acc_odr() > 0.0 && !self.int1_active(now) {
            t = t.min(next(self.acc_odr(), Self::sample_index(now, self.acc_odr())));
        }
        if self.mag_regs[0x62] & 0x01 != 0 && !self.drdy_active(now) {
            if self.mag_regs[0x60] & 3 == 0 {
                t = t.min(next(self.mag_odr(), Self::sample_index(now, self.mag_odr())));
            } else if let Some(s) = self.mag_single_until {
                t = t.min(s);
            }
        }
        t.max(now + 1)
    }

    /// I2C write to the accelerometer: first byte is the sub-address (bit 7 = auto-increment).
    pub fn acc_write(&mut self, data: &[u8]) {
        let Some((&sub, rest)) = data.split_first() else { return };
        self.acc_ptr = sub & 0x7F;
        self.acc_autoinc = sub & 0x80 != 0;
        for &b in rest {
            self.acc_write_reg(self.acc_ptr, b);
            if self.acc_autoinc {
                self.acc_ptr = (self.acc_ptr + 1) & 0x7F;
            }
        }
    }

    pub fn acc_read(&mut self, buf: &mut [u8], now: Ticks) {
        for b in buf.iter_mut() {
            *b = self.acc_read_reg(self.acc_ptr, now);
            if self.acc_autoinc {
                self.acc_ptr = (self.acc_ptr + 1) & 0x7F;
            }
        }
    }

    /// I2C write to the magnetometer (the register address increments automatically).
    pub fn mag_write(&mut self, data: &[u8], now: Ticks) {
        let Some((&sub, rest)) = data.split_first() else { return };
        self.mag_ptr = sub & 0x7F;
        for &b in rest {
            self.mag_write_reg(self.mag_ptr, b, now);
            self.mag_ptr = (self.mag_ptr + 1) & 0x7F;
        }
    }

    pub fn mag_read(&mut self, buf: &mut [u8], now: Ticks) {
        for b in buf.iter_mut() {
            *b = self.mag_read_reg(self.mag_ptr, now);
            self.mag_ptr = (self.mag_ptr + 1) & 0x7F;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bsp_configuration() {
        let mut s = Lsm303agr::new();
        // WHO_AM_I
        s.acc_write(&[0x0F]);
        let mut b = [0u8; 1];
        s.acc_read(&mut b, 0);
        assert_eq!(b[0], 0x33);
        s.mag_write(&[0x4F], 0);
        s.mag_read(&mut b, 0);
        assert_eq!(b[0], 0x40);
        // BSP: CTRL_REG1_A 0x57, CTRL_REG4_A 0x88 -> HR, +-2 g
        s.acc_write(&[0x20, 0x57]);
        s.acc_write(&[0x23, 0x88]);
        s.accel_mg = [955.0, 255.0, 265.0];
        s.acc_write(&[0x28 | 0x80]);
        let mut d = [0u8; 6];
        s.acc_read(&mut d, 1_000_000);
        let x = i16::from_le_bytes([d[0], d[1]]) >> 4;
        // BSP conversion: mg = (raw >> 4) * 98 / 100
        assert_eq!(x as i32 * 98 / 100, 954);
        // magnetometer continuous, 1.5 mgauss/LSB
        s.mag_write(&[0x60, 0x80], 0);
        s.mag_mgauss = [300.0, -150.0, 450.0];
        s.mag_write(&[0x68], 0);
        s.mag_read(&mut d, 1_000_000);
        assert_eq!(i16::from_le_bytes([d[0], d[1]]), 200);
        assert_eq!(i16::from_le_bytes([d[2], d[3]]), -100);
    }
}
