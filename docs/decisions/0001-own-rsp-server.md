# 0001 — Own GDB remote serial protocol server instead of `gdbstub`

**Status:** accepted

## Context

The CMSIS Debugger (cdt-gdb-adapter with `auxiliaryGdb: true`) opens two GDB connections to the same
port. The first controls the target: it halts, steps, sets breakpoints, loads flash. The second reads and
writes memory while the target runs, for Live Watch, the SVD peripheral view and CPU Time. pyOCD serves
both by giving the second connection a view in which the target looks stopped.

`gdbstub` models one connection that owns the target's state machine. Two connections on one target, a
"looks stopped" view for the second, `vFlash*`, `qXfer:memory-map:read` with a flash block size and
`qRcmd` with a pyOCD-like command set would all have needed work around its abstractions.

## Decision

`mbsim-gdb` implements the RSP subset the CMSIS Debugger and GDB 14 use (about 600 lines): packet framing
with run-length decoding, `qSupported`, `qXfer` for features and the memory map, `g/G/p/P`, `m/M/X`,
`Z0`–`Z4`, `vCont`, `vFlash*`, `qRcmd`, `D`, `k`. The first connection is the controller; later
connections are auxiliary. Code breakpoints use the FPB like a probe does, so the firmware sees the same
`FP_COMP` registers.

## Consequences

- Full control over the packet mix; the behaviour can follow pyOCD where the adapter depends on it (for
  example `monitor reset halt` halting at the reset vector).
- No reuse of `gdbstub`'s target-description helpers; `target.xml` is written by hand (M-profile, M-system,
  VFP features).
- The controller's disconnect ends a launched server after a 2 s grace period, because cdt-gdb-adapter
  reports "read error" when the server is gone before it closes its own socket.
