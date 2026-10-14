# 0006 — The Arm FVP as the reference for the core, recorded once

**Status:** accepted

## Context

The core must behave like a Cortex-M4F: flags, saturation, DSP SIMD, FPU rounding, NaN handling, fixed
point conversions, exception stacking. A board at hand is not always available and is slow to drive from
tests. Arm's `FVP_MPS2_Cortex-M4` is an Arm-validated model of the same core.

## Decision

`firmware/isa-test` is a bare-metal program for an MPS2 memory layout that runs 174 instruction groups
with fixed and pseudo-random operands and prints a checksum per group through semihosting. Its output on
`FVP_MPS2_Cortex-M4` 11.31 is recorded in `fixtures/isa_test.fvp.txt`; `cargo test` runs the same ELF in
the simulator and compares line by line. The FVP is needed only to re-record after the test changes.

## Consequences

- CI needs no FVP licence or Docker image.
- The comparison found one deviation (signed 16-bit `VCVT` to fixed point must sign-extend), now fixed.
- Peripheral behaviour is not covered by the FVP; it is checked by scenarios with BoardTest and by
  MicroPython, and needs measurements on the board for timing.
