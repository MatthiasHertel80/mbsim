//! Pin map of the micro:bit V2.2.1 (BSP `microbit_v2.h`); pin number = 32 * port + pin.

pub const fn pin(port: u8, n: u8) -> u8 {
    port * 32 + n
}

pub const ROW: [u8; 5] = [pin(0, 21), pin(0, 22), pin(0, 15), pin(0, 24), pin(0, 19)];
pub const COL: [u8; 5] = [pin(0, 28), pin(0, 11), pin(0, 31), pin(1, 5), pin(0, 30)];
pub const BTN_A: u8 = pin(0, 14);
pub const BTN_B: u8 = pin(0, 23);
pub const LOGO: u8 = pin(1, 4);
pub const SPEAKER: u8 = pin(0, 0);
pub const MIC_IN: u8 = pin(0, 5);
pub const RUN_MIC: u8 = pin(0, 20);
pub const I2C_INT_SCL: u8 = pin(0, 8);
pub const I2C_INT_SDA: u8 = pin(0, 16);
pub const I2C_INT_INT: u8 = pin(0, 25);
pub const I2C_EXT_SCL: u8 = pin(0, 26);
pub const I2C_EXT_SDA: u8 = pin(1, 0);
pub const UART_TX: u8 = pin(0, 6);
pub const UART_RX: u8 = pin(1, 8);
pub const RING: [u8; 3] = [pin(0, 2), pin(0, 3), pin(0, 4)];
pub const RESET: u8 = pin(0, 18);

/// Edge connector: (label, pin).
pub const EDGE: [(&str, u8); 19] = [
    ("P0", pin(0, 2)),
    ("P1", pin(0, 3)),
    ("P2", pin(0, 4)),
    ("P3", pin(0, 31)),
    ("P4", pin(0, 28)),
    ("P5", pin(0, 14)),
    ("P6", pin(1, 5)),
    ("P7", pin(0, 11)),
    ("P8", pin(0, 10)),
    ("P9", pin(0, 9)),
    ("P10", pin(0, 30)),
    ("P11", pin(0, 23)),
    ("P12", pin(0, 12)),
    ("P13", pin(0, 17)),
    ("P14", pin(0, 1)),
    ("P15", pin(0, 13)),
    ("P16", pin(1, 2)),
    ("P19", pin(0, 26)),
    ("P20", pin(1, 0)),
];

pub fn name(p: u8) -> String {
    format!("P{}.{:02}", p / 32, p % 32)
}
