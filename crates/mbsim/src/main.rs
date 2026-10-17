//! `mbsim`: BBC micro:bit V2 simulator.
//!
//! Command lines:
//! - `mbsim [run] [IMAGE] [options]`: run an image (GUI unless `--headless`).
//! - `mbsim gdbserver --cbuild-run FILE [--port N] ...`: pyOCD-compatible GDB server.
//! - FVP-compatible: `mbsim -C GDBServer.port=3333 -a image.elf [--plugin X] [-f config]`, as the
//!   CMSIS Solution `Arm-FVP` debug adapter starts a model.

mod cli;
mod control;
mod monitor;
mod register;
mod scenario;
mod uart;

use cli::Config;
use mbsim_loader::cbuild_run::RunConfig;
use mbsim_loader::Image;
use mbsim_microbit::runtime::{Cmd, Options, Simulator, Speed};
use mbsim_microbit::Machine;
use std::io::Write;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = subcommand(&args) {
        std::process::exit(code);
    }
    let cfg = match cli::parse(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mbsim: {e}\n\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    let level = if cfg.quiet {
        "error"
    } else if cfg.verbose {
        "debug"
    } else {
        "warn"
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).format_timestamp(None).init();
    if cfg.help {
        println!("{}", cli::USAGE);
        return;
    }
    if cfg.version {
        println!("mbsim {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(e) = run(cfg) {
        eprintln!("mbsim: {e}");
        std::process::exit(1);
    }
}

/// `load`, `erase`, `info` and `register-cmsis-adapter`; `None` for the simulator itself.
fn subcommand(args: &[String]) -> Option<i32> {
    let (cmd, rest) = args.split_first()?;
    let rest: Vec<String> = rest.to_vec();
    let opt = |name: &str| rest.iter().position(|a| a == name).and_then(|i| rest.get(i + 1)).cloned();
    let port = opt("--control-port").and_then(|p| p.parse().ok()).unwrap_or(control::DEFAULT_PORT);
    match cmd.as_str() {
        "load" => {
            let cfg = Config {
                cbuild_run: opt("--cbuild-run").map(Into::into),
                images: rest
                    .iter()
                    .filter(|a| {
                        !a.starts_with('-') && Some(*a) != opt("--cbuild-run").as_ref() && Some(*a) != opt("--control-port").as_ref()
                    })
                    .map(Into::into)
                    .collect(),
                ..Config::default()
            };
            let mut paths = cfg.images.clone();
            if paths.is_empty() {
                if let Some(cr) = &cfg.cbuild_run {
                    match RunConfig::load(cr) {
                        Ok(rc) => paths = rc.images(),
                        Err(e) => {
                            eprintln!("mbsim load: {e}");
                            return Some(1);
                        }
                    }
                }
            }
            let has_elf = paths.iter().any(|p| p.extension().is_some_and(|e| e == "elf" || e == "axf"));
            paths.retain(|p| !(has_elf && p.extension().is_some_and(|e| e == "hex")));
            if paths.is_empty() {
                eprintln!("mbsim load: no image (give IMAGE or --cbuild-run FILE)");
                return Some(2);
            }
            for p in &paths {
                if let Err(e) = Image::load(p) {
                    eprintln!("mbsim load: {e}");
                    return Some(1);
                }
                let abs = std::fs::canonicalize(p).unwrap_or(p.clone());
                match control::send(port, &format!("load {}", abs.display())) {
                    Some(r) if r == "ok" => println!("programmed {} into the running simulator", abs.display()),
                    Some(r) => {
                        eprintln!("mbsim load: {r}");
                        return Some(1);
                    }
                    None => println!("{}: checked; no simulator is running, the image is loaded when the simulation starts", abs.display()),
                }
            }
            Some(0)
        }
        "erase" => {
            match control::send(port, "erase") {
                Some(r) => println!("{r}"),
                None => println!("no simulator is running; a new simulation starts with erased flash"),
            }
            Some(0)
        }
        "info" => {
            println!("BBC micro:bit V2 simulator {}", env!("CARGO_PKG_VERSION"));
            println!("  device: nRF52833_xxAA (Cortex-M4F r0p1, 512 KB flash, 128 KB RAM), board MicroBit::microbit-v2 V2.2");
            if let Some(cr) = opt("--cbuild-run") {
                match RunConfig::load(std::path::Path::new(&cr)) {
                    Ok(rc) => {
                        println!("  cbuild-run: {cr}");
                        println!("  device: {}", rc.run.device.clone().unwrap_or_default());
                        println!("  board: {}", rc.run.board.clone().unwrap_or_default());
                        for i in rc.images() {
                            println!("  image: {}", i.display());
                        }
                        if let Some(s) = rc.svd() {
                            println!("  svd: {}", s.display());
                        }
                        println!("  gdb port: {}", rc.gdb_port().unwrap_or(3333));
                    }
                    Err(e) => println!("  {e}"),
                }
            }
            match control::send(port, "status") {
                Some(r) => println!("  running simulator: {r}"),
                None => println!("  no simulator running"),
            }
            Some(0)
        }
        "register-cmsis-adapter" => match register::run(&rest) {
            Ok(()) => Some(0),
            Err(e) => {
                eprintln!("mbsim register-cmsis-adapter: {e}");
                Some(1)
            }
        },
        _ => None,
    }
}

fn images_of(cfg: &Config) -> Result<Vec<Image>, String> {
    let mut paths = cfg.images.clone();
    if let Some(cr) = &cfg.cbuild_run {
        let rc = RunConfig::load(cr).map_err(|e| e.to_string())?;
        if paths.is_empty() {
            paths = rc.images();
        }
    }
    // prefer ELF over HEX of the same build
    let mut imgs = Vec::new();
    let has_elf = paths.iter().any(|p| p.extension().is_some_and(|e| e == "elf" || e == "axf"));
    for p in paths {
        if has_elf && p.extension().is_some_and(|e| e == "hex") {
            continue;
        }
        imgs.push(Image::load(&p).map_err(|e| e.to_string())?);
    }
    Ok(imgs)
}

fn run(cfg: Config) -> Result<(), String> {
    let images = images_of(&cfg)?;
    let mut m = Machine::new();
    for img in &images {
        m.load_image(img)?;
        log::info!("loaded {} ({} bytes)", img.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(), img.size());
    }
    m.power_on();
    if cfg.semihosting {
        m.semihosting = true;
    }
    let gdb_port = cfg.gdb_port;
    let opts = Options {
        speed: cfg.speed,
        log_pins: !cfg.headless,
        start_halted: gdb_port.is_some() && !cfg.reset_run,
        halt_on_lockup: false,
        end_time: cfg.max_time.map(mbsim_core::time::from_secs),
    };
    let sim = Simulator::start(m, opts);

    // GDB server: bind before anything else so the debugger can connect at once
    if let Some(port) = gdb_port {
        let addr = format!("{}:{port}", cfg.bind);
        let mut server = mbsim_gdb::Server::bind(sim.clone(), &addr).map_err(|e| format!("GDB server on {addr}: {e}"))?;
        server.monitor_ext = Some(std::sync::Arc::new(monitor::handle));
        server.exit_on_detach = !cfg.persist;
        eprintln!("GDB server listening on port {}", server.local_port());
        std::thread::Builder::new().name("gdb-server".into()).spawn(move || server.serve()).map_err(|e| e.to_string())?;
    }
    if let Some(port) = cfg.uart_tcp {
        uart::serve_tcp(sim.clone(), &cfg.bind, port)?;
    }
    if cfg.control_port != 0 {
        control::serve(sim.clone(), cfg.control_port);
    }
    if cfg.uart_stdout || cfg.headless {
        uart::to_stdout(sim.clone(), cfg.uart_stdin);
    }
    if cfg.exit_on_eot {
        uart::exit_on_eot(sim.clone());
    }
    let scenario = match &cfg.scenario {
        Some(p) => Some(scenario::schedule(&sim, scenario::load(p)?)),
        None => None,
    };
    for (t, text) in &cfg.uart_send {
        let s = sim.clone();
        let text = text.clone();
        let delay = *t;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(delay));
            s.send(Cmd::UartIn(text.into_bytes()));
        });
    }

    if cfg.mic && cfg.headless {
        log::warn!("--mic needs the window; ignored with --headless");
    }
    if !cfg.headless {
        #[cfg(feature = "gui")]
        {
            return mbsim_gui::run(sim, mbsim_gui::GuiOptions { title: cfg.title.clone(), mic: cfg.mic });
        }
        #[cfg(not(feature = "gui"))]
        log::warn!("built without the GUI; running headless");
    }
    headless_wait(&sim, &cfg, scenario)
}

fn headless_wait(
    sim: &Simulator,
    cfg: &Config,
    scenario: Option<std::sync::Arc<std::sync::Mutex<scenario::Results>>>,
) -> Result<(), String> {
    let report = |code: i32| {
        if let Some(r) = &scenario {
            let r = r.lock().unwrap();
            eprintln!("scenario: {} passed, {} failed", r.passed, r.failed);
            if r.failed > 0 {
                return 1;
            }
        }
        code
    };
    let wall = std::time::Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(20));
        if sim.status().end_reached {
            break;
        }
        if let Some(code) = sim.with(|m| m.exit_code).flatten() {
            std::thread::sleep(Duration::from_millis(50));
            let _ = std::io::stdout().flush();
            std::process::exit(report(code));
        }
    }
    let _ = std::io::stdout().flush();
    let st = sim.status();
    if cfg.stats {
        let secs = wall.elapsed().as_secs_f64();
        eprintln!(
            "simulated {:.3} s in {:.3} s, {} instructions, {:.1} MIPS, {:.1}x real time",
            mbsim_core::time::to_secs(st.time),
            secs,
            st.instret,
            st.instret as f64 / secs / 1e6,
            mbsim_core::time::to_secs(st.time) / secs
        );
    }
    for w in &st.warnings {
        eprintln!("warning: {w}");
    }
    sim.send(Cmd::Shutdown);
    let _ = Speed::Max;
    let code = report(0);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
