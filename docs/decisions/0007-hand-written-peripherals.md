# 0007 — Hand-written peripheral models, no SVD code generator

**Status:** accepted (replaces the `xtask` SVD generator of the plan)

## Context

The plan proposed generating register blocks from `nrf52833.svd`. Most of the work in a peripheral is
its behaviour (tasks, events, shorts, EasyDMA, timing), not its register list, and the nRF52 register
layout is regular: tasks at 0x000, events at 0x100, `SHORTS` at 0x200, `INTEN` at 0x300, configuration
from 0x500.

## Decision

Each peripheral is a hand-written struct with `read` and `write` on offsets. Writable-bit masks are taken
from the SVD and the product specification. The common rule "INTEN bit n belongs to the event at
0x100 + 4n" is used to route event pulses to PPI and to compute the interrupt line. Accesses to
peripherals without a model are logged once as warnings and read as zero.

## Consequences

- No build-time dependency on the SVD file or its licence.
- Register masks must be kept right by hand; tests caught wrong masks for UARTE `INTEN`, SAADC `CONFIG`
  and DWT `FUNCTION`.
