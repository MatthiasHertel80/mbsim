//! The target UART on TCP (VS Code Serial Monitor in TCP mode) and on the console.

use mbsim_microbit::runtime::{Cmd, Simulator};
use std::io::{Read, Write};
use std::net::TcpListener;

pub fn serve_tcp(sim: Simulator, bind: &str, port: u16) -> Result<(), String> {
    let addr = format!("{bind}:{port}");
    let listener = TcpListener::bind(&addr).map_err(|e| format!("UART server on {addr}: {e}"))?;
    eprintln!("UART on TCP port {port}");
    std::thread::Builder::new()
        .name("uart-tcp".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_nodelay(true);
                let rx = sim.subscribe_uart();
                let mut w = match stream.try_clone() {
                    Ok(w) => w,
                    Err(_) => continue,
                };
                std::thread::spawn(move || {
                    for data in rx {
                        if w.write_all(&data).is_err() {
                            break;
                        }
                    }
                });
                let s = sim.clone();
                let mut r = stream;
                std::thread::spawn(move || {
                    let mut buf = [0u8; 256];
                    while let Ok(n) = r.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        s.send(Cmd::UartIn(buf[..n].to_vec()));
                    }
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn to_stdout(sim: Simulator, stdin: bool) {
    let rx = sim.subscribe_uart();
    std::thread::spawn(move || {
        let mut out = std::io::stdout();
        for data in rx {
            let _ = out.write_all(&data);
            let _ = out.flush();
        }
    });
    if stdin {
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            let mut input = std::io::stdin();
            while let Ok(n) = input.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sim.send(Cmd::UartIn(buf[..n].to_vec()));
            }
        });
    }
}

/// Ends the process when the target sends EOT (0x04), as the FVP does with `uart.shutdown_on_eot`:
/// the end marker of a test run.
pub fn exit_on_eot(sim: Simulator) {
    let rx = sim.subscribe_uart();
    std::thread::spawn(move || {
        for data in rx {
            if data.contains(&0x04) {
                // let the console subscriber write what came before the EOT
                std::thread::sleep(std::time::Duration::from_millis(100));
                let _ = std::io::stdout().flush();
                let st = sim.status();
                eprintln!("\nmbsim: EOT from the target at {:.6} s, exiting", mbsim_core::time::to_secs(st.time));
                std::process::exit(0);
            }
        }
    });
}
