//! The core against Arm's reference model: the ISA test firmware must print exactly the digests
//! that FVP_MPS2_Cortex-M4 printed (fixtures/isa_test.fvp.txt).

use mbsim_core::time::from_secs;
use mbsim_loader::Image;
use mbsim_microbit::{Machine, Stop};
use std::path::Path;

fn fixture(p: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(p)
}

#[test]
fn isa_test_matches_fvp() {
    let img = Image::load(&fixture("elf/isa_test.elf")).unwrap();
    let mut m = Machine::new();
    m.load_image(&img).unwrap();
    m.power_on();
    m.semihosting = true;
    let stop = m.run_until(from_secs(30.0));
    assert_eq!(stop, Stop::Exited, "test firmware did not finish");
    assert_eq!(m.exit_code, Some(0));
    let out = String::from_utf8_lossy(&m.take_uart()).to_string();
    let sim: Vec<&str> = out.lines().filter(|l| l.contains(": 0x")).collect();
    let reference = std::fs::read_to_string(fixture("isa_test.fvp.txt")).unwrap();
    let fvp: Vec<&str> = reference.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(sim.len(), fvp.len());
    let diffs: Vec<String> = sim.iter().zip(&fvp).filter(|(a, b)| a != b).map(|(a, b)| format!("sim {a} / fvp {b}")).collect();
    assert!(diffs.is_empty(), "differences to the FVP:\n{}", diffs.join("\n"));
}

#[test]
fn boardtest_reports() {
    let img = Image::load(&fixture("elf/BoardTest.elf")).unwrap();
    let mut m = Machine::new();
    m.load_image(&img).unwrap();
    m.power_on();
    m.run_until(from_secs(1.3));
    let out = String::from_utf8_lossy(&m.take_uart()).to_string();
    assert!(out.contains("SystemCoreClock 64000000 Hz"), "{out}");
    assert!(out.contains("LSM303AGR: found (WHO_AM_I 0x33 / 0x40)"), "{out}");
    assert!(out.contains("Touch logo baseline: 194"), "{out}");
    assert!(out.contains("temp=21.00 C  acc=(0,0,-999) mg"), "{out}");
}
