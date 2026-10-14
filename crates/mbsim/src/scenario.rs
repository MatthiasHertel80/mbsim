//! Scenario files: timed inputs and expectations in simulated time (deterministic).
//!
//! ```yaml
//! - at: 1.5            # seconds of simulated time
//!   press: A           # A, B or AB
//! - at: 1.6
//!   expect-leds: ["01110", "10001", "11111", "10001", "10001"]
//! - at: 2.0
//!   release: A
//!   uart: "t"
//! - at: 2.1
//!   expect-tone: 880   # Hz, 2 % tolerance (0 = silent)
//! - at: 3.0
//!   expect-uart: "tone 880 Hz"
//!   exit: true
//! ```

use mbsim_core::time::{from_secs, ms, to_secs};
use mbsim_microbit::board::Orientation;
use mbsim_microbit::runtime::{Cmd, Simulator};
use mbsim_microbit::Machine;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Step {
    pub at: f64,
    pub press: Option<String>,
    pub release: Option<String>,
    pub touch: Option<bool>,
    pub tilt: Option<[f64; 2]>,
    pub heading: Option<f64>,
    pub temperature: Option<f64>,
    pub sound: Option<[f64; 2]>,
    pub clap: Option<f64>,
    pub uart: Option<String>,
    pub reset: Option<bool>,
    pub expect_uart: Option<String>,
    pub expect_leds: Option<Vec<String>>,
    pub expect_tone: Option<f64>,
    pub exit: Option<bool>,
}

#[derive(Default)]
pub struct Results {
    pub passed: usize,
    pub failed: usize,
}

fn buttons(s: &str) -> Vec<usize> {
    s.chars()
        .filter_map(|c| match c.to_ascii_uppercase() {
            'A' => Some(0),
            'B' => Some(1),
            _ => None,
        })
        .collect()
}

fn pattern(lit: &[[bool; 5]; 5]) -> Vec<String> {
    lit.iter().map(|r| r.iter().map(|b| if *b { '1' } else { '0' }).collect()).collect()
}

fn check(res: &Arc<Mutex<Results>>, t: f64, ok: bool, what: String) {
    let mut r = res.lock().unwrap();
    if ok {
        r.passed += 1;
        eprintln!("[{t:8.3} s] PASS {what}");
    } else {
        r.failed += 1;
        eprintln!("[{t:8.3} s] FAIL {what}");
    }
}

pub fn load(path: &std::path::Path) -> Result<Vec<Step>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_yaml_ng::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Schedules the steps on the simulator.
pub fn schedule(sim: &Simulator, steps: Vec<Step>) -> Arc<Mutex<Results>> {
    let res = Arc::new(Mutex::new(Results::default()));
    for step in steps {
        let r = res.clone();
        let at = from_secs(step.at);
        sim.send(Cmd::At(
            at,
            Box::new(move |m: &mut Machine| {
                let t = to_secs(m.now());
                let b = &mut m.sys.board;
                if let Some(p) = &step.press {
                    for i in buttons(p) {
                        b.inputs.button[i] = true;
                    }
                }
                if let Some(p) = &step.release {
                    for i in buttons(p) {
                        b.inputs.button[i] = false;
                    }
                }
                if let Some(on) = step.touch {
                    let now = m.now();
                    m.sys.board.touch_changed(now);
                    m.sys.board.inputs.logo_finger_pf = if on { 100.0 } else { 0.0 };
                }
                let b = &mut m.sys.board;
                if let Some([roll, pitch]) = step.tilt {
                    b.inputs.orientation = Orientation { roll, pitch, ..b.inputs.orientation };
                }
                if let Some(h) = step.heading {
                    b.inputs.orientation.heading = h;
                }
                if let Some(c) = step.temperature {
                    b.inputs.temperature = c;
                }
                if let Some([mv, hz]) = step.sound {
                    b.inputs.sound_mv = mv;
                    b.inputs.sound_hz = hz;
                }
                if let Some(mv) = step.clap {
                    let now = m.now();
                    m.sys.board.clap(now, mv);
                }
                if let Some(u) = &step.uart {
                    m.uart_send(u.as_bytes());
                }
                if step.reset == Some(true) {
                    m.reset_button();
                }
                let now = m.now();
                m.sys.inputs_changed(now);
                if let Some(e) = &step.expect_uart {
                    let text = m.uart_text();
                    check(&r, t, text.contains(e.as_str()), format!("UART contains {e:?}"));
                }
                if let Some(e) = &step.expect_leds {
                    let lit = m.sample_leds(ms(15));
                    let got = pattern(&lit);
                    let want: Vec<String> = e.iter().map(|s| s.replace(['#', 'x', 'X'], "1").replace('.', "0")).collect();
                    check(&r, t, got == want, format!("LEDs {want:?} (got {got:?})"));
                }
                if let Some(hz) = step.expect_tone {
                    let f = m.measure_speaker_hz(ms(60));
                    let ok = if hz == 0.0 { f == 0.0 } else { (f - hz).abs() <= hz * 0.02 };
                    check(&r, t, ok, format!("speaker {hz} Hz (got {f:.1} Hz)"));
                }
                if step.exit == Some(true) {
                    let failed = r.lock().unwrap().failed;
                    m.exit_code = Some(if failed > 0 { 1 } else { 0 });
                }
            }),
        ));
    }
    res
}
