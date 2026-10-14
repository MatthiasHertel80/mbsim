# 0003 — 512 MHz tick time base and an event scheduler

**Status:** accepted

## Context

The board has a 64 MHz core clock, 16 MHz peripheral clocks (TIMER, PWM, TWIM, UARTE baud generators) and
a 32.768 kHz low-frequency clock (RTC). Polling peripherals every cycle would cost more than the CPU.

## Decision

Simulated time is a `u64` count of 512 MHz ticks: 8 per CPU cycle, 32 per 16 MHz cycle. Rates that do not
divide it (32.768 kHz, baud rates, I2C bit rates) use `FracPeriod`, which accumulates the remainder so
edges never drift. Each peripheral owns scheduler slots and computes its counters from the time of the
last start; the scheduler wakes it only for its next event. A core in WFI/WFE skips to the next event.

## Consequences

- An idle firmware advances simulated time at thousands of times real time.
- `u64` ticks cover 1,142 years.
- A scenario step runs at an exact simulated time, so results do not depend on the host.
