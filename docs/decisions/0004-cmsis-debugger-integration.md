# 0004 — Three ways into the CMSIS Debugger; the Arm-FVP form first

**Status:** accepted

## Context

CMSIS Solution generates `launch.json` and `tasks.json` from the debug adapter that a target-type names.
It knows pyOCD, J-Link, ULINK and the Arm FVPs. A simulator that needs its own adapter needs an installer
that patches the extension's templates, and a new extension release can undo that.

## Decision

1. **Arm-FVP form (mode A).** `mbsim` accepts the FVP command line the `Arm-FVP` adapter template produces
   (`-C GDBServer.port=N`, `-a image`, `-f file`; `--plugin` and `-D` are ignored). Its own options are
   `-C` parameters whose last component is `headless`, `speed`, `uart_port` or `persist`, for example
   `-C sim.headless=1`; other FVP parameters are ignored.
   A csolution selects it with `debugger: name: Arm-FVP, model: <path to mbsim>`. Nothing is installed.
2. **Own adapter `microbit-v2-sim` (mode B).** `cmsis/microbit-v2-sim.adapter.json` and the
   `debug-adapters.yml` entry, installed by `mbsim register-cmsis-adapter --apply` with a backup.
   `mbsim gdbserver` takes pyOCD's options, so the template differs from pyOCD's only in the server name.
3. **Hand-written launch.json (mode C)** with `"updateConfiguration": "manual"`, keeping the `cmsis` node
   so CPU Time and the SVD view still find the cbuild-run file.

## Consequences

- Mode A was verified with the CMSIS Debugger: Load & Debug stops at `main`.
- The FVP template sets `auxiliaryGdb: false`, so Live Watch needs mode B or C.
- Mode B depends on the template layout of the extension; the installer only adds files and refuses to
  overwrite foreign ones.
