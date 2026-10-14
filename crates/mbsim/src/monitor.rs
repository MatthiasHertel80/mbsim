//! `monitor sim ...` commands of the GDB server.

use mbsim_core::time::to_secs;
use mbsim_microbit::board::Orientation;
use mbsim_microbit::runtime::{Cmd, Input, Simulator, Speed};

pub fn handle(sim: &Simulator, cmd: &str) -> Option<String> {
    let w: Vec<&str> = cmd.split_whitespace().collect();
    if w.first() != Some(&"sim") {
        return None;
    }
    let f = |s: &str| s.parse::<f64>().ok();
    Some(match &w[1..] {
        ["time"] => format!("{:.6} s\n", to_secs(sim.status().time)),
        ["stats"] => {
            let s = sim.status();
            format!(
                "time {:.6} s, {} instructions, {:.1} MIPS, {:.2}x real time, {} resets\n",
                to_secs(s.time),
                s.instret,
                s.mips,
                s.realtime_factor,
                s.resets
            )
        }
        ["speed", "max"] => {
            sim.send(Cmd::Speed(Speed::Max));
            "speed: max\n".into()
        }
        ["speed", "realtime"] => {
            sim.send(Cmd::Speed(Speed::Realtime(1.0)));
            "speed: real time\n".into()
        }
        ["speed", x] => match f(x) {
            Some(v) if v > 0.0 => {
                sim.send(Cmd::Speed(Speed::Realtime(v)));
                format!("speed: {v}x\n")
            }
            _ => "usage: sim speed <factor|max|realtime>\n".into(),
        },
        ["button", b, action] => {
            let idx = match *b {
                "a" | "A" => 0,
                "b" | "B" => 1,
                _ => return Some("usage: sim button a|b press|release\n".into()),
            };
            let pressed = matches!(*action, "press" | "down" | "1");
            sim.send(Cmd::Input(Input::Button(idx, pressed)));
            format!("button {} {}\n", b.to_uppercase(), if pressed { "pressed" } else { "released" })
        }
        ["touch", s] => {
            let on = matches!(*s, "on" | "1");
            sim.send(Cmd::Input(Input::Touch(if on { 100.0 } else { 0.0 })));
            format!("logo {}\n", if on { "touched" } else { "released" })
        }
        ["tilt", r, p] => match (f(r), f(p)) {
            (Some(roll), Some(pitch)) => {
                let heading = sim.status().inputs.map(|i| i.orientation.heading).unwrap_or(0.0);
                sim.send(Cmd::Input(Input::Orientation(Orientation { heading, roll, pitch })));
                format!("roll {roll} pitch {pitch}\n")
            }
            _ => "usage: sim tilt <roll> <pitch>\n".into(),
        },
        ["heading", h] => match f(h) {
            Some(heading) => {
                let o = sim.status().inputs.map(|i| i.orientation).unwrap_or_default();
                sim.send(Cmd::Input(Input::Orientation(Orientation { heading, ..o })));
                format!("heading {heading}\n")
            }
            None => "usage: sim heading <degrees>\n".into(),
        },
        ["temp", t] => match f(t) {
            Some(c) => {
                sim.send(Cmd::Input(Input::Temperature(c)));
                format!("temperature {c} C\n")
            }
            None => "usage: sim temp <celsius>\n".into(),
        },
        ["sound", mv, rest @ ..] => match f(mv) {
            Some(mv) => {
                let hz = rest.first().and_then(|x| f(x)).unwrap_or(440.0);
                sim.send(Cmd::Input(Input::Sound { mv, hz }));
                format!("sound {mv} mV {hz} Hz\n")
            }
            None => "usage: sim sound <mV> [Hz]\n".into(),
        },
        ["clap"] => {
            sim.send(Cmd::Input(Input::Clap(13.0)));
            "clap\n".into()
        }
        ["uart", rest @ ..] => {
            let mut text = rest.join(" ");
            text.push('\n');
            sim.send(Cmd::UartIn(text.into_bytes()));
            "sent\n".into()
        }
        ["leds"] => sim
            .with(|m| {
                let mut s = String::new();
                for r in 0..5 {
                    for c in 0..5 {
                        s.push(if m.sys.board.led_on(r, c) { '#' } else { '.' });
                    }
                    s.push('\n');
                }
                s
            })
            .unwrap_or_default(),
        ["warnings"] => {
            let w = sim.status().warnings;
            if w.is_empty() {
                "no warnings\n".into()
            } else {
                w.join("\n") + "\n"
            }
        }
        _ => "unknown sim command (monitor help)\n".into(),
    })
}
