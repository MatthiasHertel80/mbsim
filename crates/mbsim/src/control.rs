//! Local control port of a running simulator (text lines on 127.0.0.1): `load <file>`,
//! `erase`, `reset`, `status`. `mbsim load` uses it to program a board that is already running
//! (CMSIS Load after CMSIS Run).

use mbsim_core::time::to_secs;
use mbsim_loader::Image;
use mbsim_microbit::runtime::{Cmd, Simulator};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 4460;

pub fn serve(sim: Simulator, port: u16) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::info!("control port {port} not available: {e}");
            return;
        }
    };
    std::thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let sim = sim.clone();
                std::thread::spawn(move || handle(sim, stream));
            }
        })
        .ok();
}

fn handle(sim: Simulator, stream: TcpStream) {
    let mut out = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        let (cmd, arg) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
        let reply = match cmd {
            "load" => match Image::load(std::path::Path::new(arg)) {
                Ok(img) => match sim.load(img, false) {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("error {e}"),
                },
                Err(e) => format!("error {e}"),
            },
            "erase" => {
                sim.with(|m| {
                    m.erase_all();
                    m.system_reset(0);
                });
                "ok".into()
            }
            "reset" => {
                sim.send(Cmd::Input(mbsim_microbit::runtime::Input::ResetButton));
                "ok".into()
            }
            "status" => {
                let s = sim.status();
                format!("ok time={:.6} halted={:?} debugger={}", to_secs(s.time), s.halted, s.debugger)
            }
            _ => "error unknown command".into(),
        };
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}

/// Sends one command to a running simulator; `None` when none listens.
pub fn send(port: u16, line: &str) -> Option<String> {
    let mut s = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(300)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(30))).ok()?;
    writeln!(s, "{line}").ok()?;
    let mut r = BufReader::new(s);
    let mut reply = String::new();
    r.read_line(&mut reply).ok()?;
    Some(reply.trim().to_string())
}
