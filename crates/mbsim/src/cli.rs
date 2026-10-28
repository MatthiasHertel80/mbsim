//! Command-line parsing: native options, pyOCD-style `gdbserver` and FVP-style model options.

use mbsim_microbit::runtime::Speed;
use std::path::PathBuf;

pub const USAGE: &str = "\
usage:
  mbsim [run] [IMAGE...] [options]          run images (.elf/.hex/.bin)
  mbsim load [IMAGE | --cbuild-run FILE]    program a running simulator (control port)
  mbsim erase                               erase the flash of a running simulator
  mbsim info [--cbuild-run FILE]            target information
  mbsim register-cmsis-adapter [--extension DIR] [--toolbox DIR] [--apply --backup-dir DIR]
                                            add the mbsim debug adapter to CMSIS Solution
  mbsim gdbserver [--cbuild-run FILE] [--port N] [--telnet-port N] [--persist]
                                            GDB server with pyOCD-compatible options
  mbsim -C GDBServer.port=N -a IMAGE [...]  FVP-compatible options (CMSIS Arm-FVP debug adapter):
                                            -C, -f FILE, -a, --simlimit, --stat; -C parameters ending in
                                            disable-visualisation, shutdown_on_eot, out_file (-), speed,
                                            uart_port, semihosting-enable, persist or mic set mbsim options

options:
  --cbuild-run FILE     images (and GDB port) from a CMSIS-Toolbox *.cbuild-run.yml
  --headless            no window; the target UART goes to stdout
  --gdb-port N          start the GDB server on port N (the core waits halted for the debugger)
  --reset-run           with a GDB port: run immediately instead of waiting halted
  --bind ADDR           address of the servers (default 127.0.0.1)
  --uart-tcp N          serve the target UART on TCP port N (VS Code Serial Monitor: TCP mode)
  --uart-stdout         print the target UART on stdout (default with --headless)
  --uart-stdin          send stdin to the target UART
  --mic                 the host microphone feeds the board's microphone (window)
  --send SECONDS:TEXT   send TEXT to the target UART after SECONDS of wall time
  --speed X|max         simulation speed factor (default 1 = real time)
  --time SECONDS        stop after this simulated time (headless)
  --exit-on-eot         exit when the target UART sends EOT (0x04), like the FVP's uart.shutdown_on_eot
  --semihosting         serve semihosting calls (BKPT 0xAB) on the console
  --scenario FILE       timed inputs and expectations (YAML, simulated time); exit code 1 on failure
  --persist             keep running when the debugger disconnects
  --stats               print statistics at the end
  --control-port N      local control port for mbsim load (default 4460, 0 = off)
  -q, --quiet / -v, --verbose
";

#[derive(Debug, Clone)]
pub struct Config {
    pub images: Vec<PathBuf>,
    pub cbuild_run: Option<PathBuf>,
    pub headless: bool,
    pub gdb_port: Option<u16>,
    pub bind: String,
    pub uart_tcp: Option<u16>,
    pub uart_stdout: bool,
    pub uart_stdin: bool,
    pub uart_send: Vec<(f64, String)>,
    pub speed: Speed,
    pub max_time: Option<f64>,
    pub persist: bool,
    pub reset_run: bool,
    pub semihosting: bool,
    pub stats: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub help: bool,
    pub version: bool,
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub title: String,
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub mic: bool,
    pub scenario: Option<PathBuf>,
    pub control_port: u16,
    pub exit_on_eot: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            images: Vec::new(),
            cbuild_run: None,
            headless: false,
            gdb_port: None,
            bind: "127.0.0.1".into(),
            uart_tcp: None,
            uart_stdout: false,
            uart_stdin: false,
            uart_send: Vec::new(),
            speed: Speed::Realtime(1.0),
            max_time: None,
            persist: false,
            reset_run: false,
            semihosting: false,
            stats: false,
            quiet: false,
            verbose: false,
            help: false,
            version: false,
            title: "mbsim".into(),
            mic: false,
            scenario: None,
            control_port: crate::control::DEFAULT_PORT,
            exit_on_eot: false,
        }
    }
}

fn value(args: &[String], i: &mut usize, name: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i).cloned().ok_or_else(|| format!("{name} needs a value"))
}

/// An FVP boolean parameter: 1 or true.
fn on(v: &str) -> bool {
    v == "1" || v.eq_ignore_ascii_case("true")
}

fn port(s: &str) -> Result<u16, String> {
    s.trim().trim_matches('"').parse().map_err(|_| format!("invalid port '{s}'"))
}

fn speed(s: &str) -> Result<Speed, String> {
    if s == "max" {
        Ok(Speed::Max)
    } else if s == "realtime" {
        Ok(Speed::Realtime(1.0))
    } else {
        s.parse::<f64>().map(Speed::Realtime).map_err(|_| format!("invalid speed '{s}'"))
    }
}

/// Parses the arguments; FVP form when `-C`, `-a` or `--plugin` appear.
pub fn parse(raw: Vec<String>) -> Result<Config, String> {
    let args: Vec<String> = raw.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    let fvp = args.iter().any(|a| a == "-C" || a == "-a" || a == "--plugin" || a == "-f" || a.starts_with("--application"));
    if fvp {
        return parse_fvp(&args);
    }
    let mut c = Config::default();
    let mut i = 0;
    let mut gdbserver = false;
    if args.first().map(|s| s.as_str()) == Some("gdbserver") {
        gdbserver = true;
        i = 1;
    } else if args.first().map(|s| s.as_str()) == Some("run") {
        i = 1;
    }
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => c.help = true,
            "-V" | "--version" => c.version = true,
            "--cbuild-run" => c.cbuild_run = Some(value(&args, &mut i, a)?.into()),
            "--headless" => c.headless = true,
            "--gdb-port" | "--port" | "-p" => c.gdb_port = Some(port(&value(&args, &mut i, a)?)?),
            "--telnet-port" | "-T" | "--uart-tcp" => c.uart_tcp = Some(port(&value(&args, &mut i, a)?)?),
            "--bind" => c.bind = value(&args, &mut i, a)?,
            "--allow-remote" => c.bind = "0.0.0.0".into(),
            "--uart-stdout" => c.uart_stdout = true,
            "--uart-stdin" => c.uart_stdin = true,
            "--mic" => c.mic = true,
            "--send" => {
                let v = value(&args, &mut i, a)?;
                let (t, text) = v.split_once(':').ok_or("--send SECONDS:TEXT")?;
                c.uart_send.push((t.parse().map_err(|_| "--send SECONDS:TEXT")?, text.replace("\\n", "\n").replace("\\r", "\r")));
            }
            "--speed" => c.speed = speed(&value(&args, &mut i, a)?)?,
            "--time" => c.max_time = Some(value(&args, &mut i, a)?.parse().map_err(|_| "--time SECONDS")?),
            "--exit-on-eot" => c.exit_on_eot = true,
            "--persist" => c.persist = true,
            "--reset-run" => c.reset_run = true,
            "--semihosting" | "-S" => c.semihosting = true,
            "--stats" => c.stats = true,
            "--scenario" => c.scenario = Some(value(&args, &mut i, a)?.into()),
            "--control-port" => c.control_port = port(&value(&args, &mut i, a)?)?,
            "-q" | "--quiet" => c.quiet = true,
            "-v" | "--verbose" => c.verbose = true,
            // pyOCD options that do not apply to the simulator
            "--probe" | "--uid" | "-u" | "--log" | "-L" | "--target" | "-t" | "--frequency" | "-f" | "-O" | "--config" | "--pack"
            | "--elf" | "--connect" | "-M" | "--erase" | "--script" => {
                i += 1;
            }
            "--no-config" | "--vector-catch" | "--step-int" | "--no-wait" => {}
            _ if a.starts_with('-') => log::warn!("ignoring unknown option {a}"),
            _ => c.images.push(PathBuf::from(a)),
        }
        i += 1;
    }
    if gdbserver && c.gdb_port.is_none() {
        let p = c
            .cbuild_run
            .as_ref()
            .and_then(|f| mbsim_loader::cbuild_run::RunConfig::load(f).ok())
            .and_then(|r| r.gdb_port())
            .unwrap_or(3333);
        c.gdb_port = Some(p);
    }
    Ok(c)
}

/// FVP command line: `-C key=value`, `-f config`, `-a [instance=]image`, `--plugin`, `-D`, ...
fn parse_fvp(args: &[String]) -> Result<Config, String> {
    let mut c = Config::default();
    let mut params: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-C" | "--parameter" => params.push(value(args, &mut i, a)?),
            "-f" | "--config-file" => {
                let f = value(args, &mut i, a)?;
                let text = std::fs::read_to_string(&f).map_err(|e| format!("{f}: {e}"))?;
                for line in text.lines() {
                    // `instance.parameter=value  # comment`, as the FVP writes and reads them
                    let l = line.split_once('#').map_or(line, |(l, _)| l).trim();
                    if !l.is_empty() {
                        params.push(l.to_string());
                    }
                }
            }
            "-a" | "--application" => {
                let v = value(args, &mut i, a)?;
                let file = v.split_once('=').map(|(_, f)| f.to_string()).unwrap_or(v);
                c.images.push(PathBuf::from(file.trim_matches('"')));
            }
            "--plugin" | "--data" | "-I" | "--iris-port" | "--cpulimit" | "--cyclelimit" => {
                i += 1;
            }
            "--simlimit" => c.max_time = Some(value(args, &mut i, a)?.parse().map_err(|_| "--simlimit SECONDS")?),
            "--headless" => c.headless = true,
            "--mic" => c.mic = true,
            "--stat" => c.stats = true,
            "-h" | "--help" => c.help = true,
            "--version" => c.version = true,
            "-q" | "--quiet" => c.quiet = true,
            _ if a.starts_with('-') => {}
            _ => c.images.push(PathBuf::from(a)),
        }
        i += 1;
    }
    for p in params {
        let Some((k, v)) = p.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim().trim_matches('"'));
        let key = k.rsplit('.').next().unwrap_or(k);
        match key {
            "port" if k.contains("GDBServer") => c.gdb_port = Some(port(v)?),
            "uart_port" | "uart-tcp" => c.uart_tcp = Some(port(v)?),
            "disable-visualisation" | "headless" => c.headless = on(v),
            "shutdown_on_eot" => c.exit_on_eot = on(v),
            "out_file" => c.uart_stdout = v == "-",
            "semihosting-enable" => c.semihosting = on(v),
            "speed" => c.speed = speed(v)?,
            "persist" => c.persist = on(v),
            "mic" => c.mic = on(v),
            _ => log::info!("ignoring FVP parameter {k}={v}"),
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn fvp_form() {
        let c = parse(v(&["-D", "--plugin", "/x/GDBServer.so\n", "-C", "GDBServer.port=3333\n", "-a", "out/app.elf"])).unwrap();
        assert_eq!(c.gdb_port, Some(3333));
        assert_eq!(c.images, vec![PathBuf::from("out/app.elf")]);
    }

    #[test]
    fn fvp_adapter_template() {
        // what the Arm-FVP template of CMSIS Solution passes for `args: -C sim.uart_port=4444 -C sim.mic=1`
        let c = parse(v(&[
            "-D",
            "--plugin",
            "/GDBServer.so\n",
            "-C",
            "GDBServer.port=3333\n",
            "-C\n",
            "sim.uart_port=4444\n",
            "-C\n",
            "sim.mic=1\n",
            "-a\n",
            "/work/out/BoardTest/microbit-v2/Debug/BoardTest.elf",
        ]))
        .unwrap();
        assert_eq!(c.gdb_port, Some(3333));
        assert_eq!(c.uart_tcp, Some(4444));
        assert!(c.mic);
        assert_eq!(c.images, vec![PathBuf::from("/work/out/BoardTest/microbit-v2/Debug/BoardTest.elf")]);
    }

    #[test]
    fn fvp_config_file() {
        let f = std::env::temp_dir().join(format!("mbsim-cli-test-{}.txt", std::process::id()));
        std::fs::write(
            &f,
            "# comment\nboard.visualisation.disable-visualisation=1   # no window\nboard.uart0.shutdown_on_eot=1\nboard.uart0.out_file=-\n",
        )
        .unwrap();
        let c = parse(v(&[
            "-f",
            f.to_str().unwrap(),
            "-C",
            "board.visualisation.disable-visualisation=0",
            "-a",
            "app.elf",
            "--simlimit",
            "60",
        ]))
        .unwrap();
        std::fs::remove_file(&f).ok();
        assert!(!c.headless, "a later -C overrides the config file");
        assert!(c.exit_on_eot);
        assert!(c.uart_stdout);
        assert_eq!(c.max_time, Some(60.0));
    }

    #[test]
    fn pyocd_form() {
        let c = parse(v(&["gdbserver", "--probe", "cmsisdap:", "--port", "3334", "--quiet", "--log", "*.x=info"])).unwrap();
        assert_eq!(c.gdb_port, Some(3334));
        assert!(c.quiet);
    }
}
