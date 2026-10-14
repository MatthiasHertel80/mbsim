# 0002 — Interpreter with a decode cache, no JIT

**Status:** accepted

## Context

The board runs at 64 MHz. Real time with margin for a debugger, the GUI and audio needs a few tens of
MIPS. Bit-exact behaviour, precise exceptions, watchpoints and single-step matter more than peak speed.

## Decision

`mbsim-cpu` decodes each Thumb-2 instruction into an `Instr` enum and executes it with a `match`.
Decoded flash instructions are kept in a direct-mapped cache indexed by halfword address; the cache is
invalidated by a generation counter that NVMC writes and erases increment. RAM code is decoded each time.

## Consequences

- About 90 MIPS in a release build on Apple silicon, 16× real time for BoardTest; sleeping cores cost
  nothing (see [0003](0003-time-base-and-scheduler.md)).
- Portable to every host without code generation, W^X handling or per-architecture back ends.
- A JIT can come later behind the same `Bus` trait if a workload needs it.
