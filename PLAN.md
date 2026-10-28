# Plan: `mbsim` — a simulator that works with the BBC micro:bit V2, with a CMSIS-Debugger GDB server

> **Status (2026-10-13):** M1, M2 and M4 are implemented; M3 without the random lockstep corpus and
> CoreValidation (the core is checked by a 174-group ISA test against the FVP); M5 without snapshot tests.
> From M8: the phase-2 peripherals (RTC, GPIOTE, PPI, EGU, RNG, WDT) and the interface-MCU I2C protocol.
> Open: M6 calibration against a real board (first steps, item 3), M7 signing and installers, the
> `--strict` mode, record and replay, board-state persistence.
> Decisions that differ from this plan are in [docs/decisions/](docs/decisions/); usage is in
> [README.md](README.md).

## Context

The Board Support Pack `MicroBit::MicroBit_V2_BSP` 0.1.0
([MatthiasHertelArm/MicroBit_V2_BSP](https://github.com/MatthiasHertelArm/MicroBit_V2_BSP)) describes the BBC micro:bit V2.2 (Nordic nRF52833, Cortex-M4F at
64 MHz, 512 KB Flash, 128 KB RAM). It ships the CMSIS-Driver VIO, the `Board Support:*` drivers behind
`Board/Include/microbit_v2.h`, the Blinky and BoardTest examples, the V2.2.1 schematic, and knowledge units
with values measured on a real board.

This project builds a simulator of exactly that board:

- An **instruction-set simulator** of the nRF52833's Cortex-M4F (ARMv7E-M, DSP extension, FPv4-SP-D16, MPU,
  NVIC, SysTick, debug components).
- **Register-level models** of every nRF52833 peripheral that the BSP drivers and the Nordic `SystemInit` touch.
- **Board-level models** of the parts on the PCB: LED matrix, buttons, touch logo, speaker, microphone,
  LSM303AGR, UART bridge of the interface MCU.
- A **GDB Remote Serial Protocol server** that the Arm CMSIS Debugger (VS Code) drives as it drives pyOCD with
  the on-board DAPLink: Load & Debug, attach, breakpoints, Live Watch, CPU Time, Peripheral Inspector and
  Serial Monitor.
- A **GUI** that shows the board as it looks: the front and back of the PCB, the LEDs lit with their real duty
  cycle, plus the inputs a user can operate.
- Written in **Rust**, with releases for Windows, macOS and Linux on x86-64 and arm64.

The unmodified ELF files built from the BSP examples (`Blinky.elf`, `BoardTest.elf`, GCC and Arm Compiler 6)
are the first acceptance tests. The simulator also lets the CMSIS Developer Assistant debug tools and CI run
without hardware.

### What "exactly this board" means (the fidelity contract)

| Level | Contract | Verified against |
|:--|:--|:--|
| Core | Bit-exact architectural state after every instruction, including flags, FPSCR, exception entry and return, faults and lockup. | Arm FVP_MPS2_Cortex-M4 (lockstep), the real nRF52833, CMSIS-Core CoreValidation |
| Device | Reset values, access types, task/event/short/interrupt semantics and EasyDMA rules of every modelled register, per the nRF52833 Product Specification and `nrf52833.svd`. | Register tests generated from the SVD; register captures from the real board |
| Board | Netlist of schematic V2.2.1: pull-ups, LED polarity, RC times, sensor mounting, analog bias. | BSP knowledge units and fresh measurements on the board |
| Timing | Peripheral timing exact to the PS formulas in their own clocks (16 MHz, 1 MHz, 32.768 kHz, baud rates). CPU timing cycle-approximate (Cortex-M4 TRM cycle table plus a flash/cache model), calibrated with `DWT->CYCCNT` on the board. | Calibration loops on the board, the touch charge-time count (194 untouched) |

Non-goals for 1.0:

- RADIO/BLE, NFCT, USBD (not wired on the micro:bit), CCM/AAR/ECB. These register blocks exist and read as zero.
- Emulating the DAPLink firmware beyond what the target sees: UART bridge, reset, LEDs.
- Electrical simulation beyond a resistive net model.
- A JIT.
- Cycle-exact bus contention.

## Sources of truth

| Topic | Source |
|:--|:--|
| Pin map, driver behaviour | BSP `Board/Include/microbit_v2.h`, `Board/microbit_*.c`, `Board/vio_microbit_v2.c` |
| Measured board values | BSP `.knowledge/units/concept-microbit-v2-hardware.md`, `concept-nrf52-peripheral-recipes.md`, `concept-lsm303agr-i2c.md`, `Documents/OVERVIEW.md` (validation section) |
| Board netlist | `Documents/MicroBit_V2.2.1_nRF52820_schematic.pdf` (CC BY-SA 4.0), github.com/microbit-foundation/microbit-v2-hardware |
| Device registers | `~/.cache/arm/packs/NordicSemiconductor/nRF_DeviceFamilyPack/8.44.1/SVD/nrf52833.svd` (BSD-3-Clause, may be vendored), `Device/Include/nrf52833*.h` |
| Device behaviour | nRF52833 Product Specification v1.x (Nordic), nRF52833 errata |
| Startup accesses | `Examples/*/RTE/Device/nRF52833_xxAA/system_nrf52.c`, `gcc_startup_nrf52833.S` (errata workarounds, FICR/UICR reads, APPROTECT, FPU enable) |
| Core | ARMv7-M Architecture Reference Manual (DDI 0403E), Cortex-M4 TRM (DDI 0439), CoreSight/ADIv5 (for the pyOCD probe mode) |
| Sensors | ST LSM303AGR datasheet (DocID027765), Knowles SPU0410LR5H-QB datasheet, MLT-8530 datasheet |
| Debugger contract | CMSIS Solution 1.72.0 `templates/debug/*.adapter.json` and `debug-adapters.yml`; CMSIS-Toolbox 2.15.0 `etc/debug-adapters.yml`; CMSIS Debugger 1.8.0 (`dist/main.js`); cdt-gdb-adapter 2.9.1 (`GDBTargetDebugSession.js`); pyOCD 0.45.1 `gdbserver/gdbserver.py` (reference behaviour) |
| GDB | GDB 15.2 bundled with the CMSIS Debugger; GDB RSP documentation (Remote Protocol appendix) |
| Prior art | an RSP server in Python with an FVP Iris backend (usable as a lockstep reference); an installer script that registers a custom CMSIS debug adapter |

## Requirements and acceptance

| ID | Requirement | Accepted when |
|:--|:--|:--|
| R1 | Runs the unmodified BSP example images | `Blinky.elf` and `BoardTest.elf` (GCC and AC6) run. BoardTest prints the same lines as on the board: banner with `SystemCoreClock 64000000`, `LSM303AGR: found (WHO_AM_I 0x33 / 0x40)`, touch baseline 194 ±15 %, mic idle level and one report per second. Every key command works. |
| R2 | CMSIS Debugger compatible | With the stock CMSIS Debugger 1.8 and its GDB 15.2: Load & Debug stops at `main`; attach works; up to 6 hardware breakpoints plus software breakpoints; 4 watchpoints; step in/over/out; Reset Target (`monitor reset halt`); Live Watch, Memory Inspector and Peripheral Inspector work while the target runs; CPU Time shows; Core Peripherals view decodes faults; Serial Monitor shows the UART. |
| R3 | GUI shows the board optics | Front and back of the board; LED brightness follows the real duty cycle; microphone LED; power and USB LEDs. Buttons, logo, reset, tilt, compass, sound and temperature inputs are operable. |
| R4 | All major OSs | Signed release builds for Windows x64/arm64, macOS arm64/x64, Linux x64/arm64 (glibc ≥ 2.31, X11 and Wayland). |
| R5 | Headless and deterministic | `--headless` with scenario files (timed inputs, assertions on UART, LEDs and pins). Exit codes for CI. Record and replay reproduce a run bit for bit. |
| R6 | Speed | BoardTest runs in real time on a 2020-class laptop. At least 100 MIPS for compute-bound code on Apple M-series (target, measured with CoreMark). |

## Architecture

```
┌─ mbsim process ──────────────────────────────────────────────────────────────────┐
│                                                                                  │
│  GUI thread (eframe/egui + wgpu)          sim thread (owns the Machine)          │
│  ┌────────────────────────────┐  inputs   ┌──────────────────────────────────┐   │
│  │ board optics, front/back   │─────────► │ Cortex-M4F core   mbsim-cpu      │   │
│  │ LED integrator, panels     │           │ nRF52833 + bus    mbsim-nrf52833 │   │
│  │ UART terminal, sim control │◄───────── │ board netlist     mbsim-microbit │   │
│  └────────────────────────────┘  pin log, │ scheduler         512 MHz ticks  │   │
│                                  UART,    └────────┬────────────────┬────────┘   │
│  audio thread (cpal) ◄────────── snapshots,        │ requests       │ bytes      │
│                                  speaker edges     │ between quanta │            │
│                          ┌─────────────────────────┴───────┐ ┌──────┴────────┐   │
│                          │ debug service: run control,     │ │ UART bridge   │   │
│                          │ memory and register access      │ │               │   │
│                          └───────┬────────────────┬────────┘ └──────┬────────┘   │
│                           RSP :3333         RSP :3333        TCP :4444 / PTY     │
│                           controller        auxiliary                            │
└──────────────────────────────────┼────────────────┼─────────────────┼────────────┘
                                   │                │                 │
                      arm-none-eabi-gdb    2nd GDB (Live Watch,   VS Code Serial
                      (CMSIS Debugger)     CPU Time, views)       Monitor
```

- **One process** holds the GUI and the GDB server, as an FVP shows its visualisation while being debugged. The
  GDB port is bound **before** the window opens, so GDB never waits on the GUI. `--headless` drops the window.
- The **sim thread** owns all simulation state; nothing else touches it. It runs the core in quanta up to the next
  scheduled event, the next pending debug request, or at most 50 µs of simulated time (so memory reads while
  running answer within that time). Between quanta it applies inputs and serves debug requests.
- The **GUI** never polls registers. The sim thread publishes:
  - a time-stamped pin-change log of all 48 GPIOs (lock-free SPSC ring);
  - UART bytes;
  - a low-rate state snapshot (sensors, peripheral summary, sim time, MIPS).
- **Time**: one `u64` tick of 1/512 MHz (1.953125 ns), the least common multiple of 64 MHz and 32.768 kHz.
  Ticks per period: CPU cycle 8, 16 MHz 32, 1 MHz 512, LFCLK 15625. A `u64` covers more than 1000 years.
  Baud and I2C bit times that are not whole ticks accumulate their fraction, so the error stays below one
  tick.
- **Pacing**: real time (default), N× or as fast as possible. `WFI`/`WFE` with nothing pending jumps straight
  to the next event, which makes the BSP's `microbit_delay_ms` (`__WFI` loop) nearly free.
- **Determinism**: given the image, the board state and the input log (inputs stamped in sim time), a run is
  reproducible. Host time enters only through pacing, never through the model.

## Workspace layout

```
microbit-v2-sim/
  Cargo.toml                 # workspace, pinned toolchain (rust-toolchain.toml)
  crates/
    mbsim-core/              # SimTime, Scheduler, Bus/Device traits, event log, net solver, input log
    mbsim-cpu/               # ARMv7E-M + FPv4-SP decoder/interpreter, exceptions, SCS (NVIC, SCB, SysTick,
                             #   MPU, FPU regs), DCB, DWT, FPB, ITM, ROM table
    mbsim-nrf52833/          # memory map, NVMC/FICR/UICR, CLOCK, POWER, GPIO, TIMER, UARTE, TWIM, SAADC,
                             #   PWM, TEMP, … ; EasyDMA; register metadata generated from nrf52833.svd
    mbsim-microbit/          # board netlist and parts: LED matrix, buttons, touch, speaker, microphone,
                             #   LSM303AGR, interface MCU (UART bridge, reset, I2C stub)
    mbsim-loader/            # ELF/HEX/BIN, *.cbuild-run.yml, board-state directory (flash + UICR)
    mbsim-gdb/               # RSP server, multi-client, target.xml, memory map, vFlash, monitor commands,
                             #   FVP-compatible and pyOCD-compatible command lines
    mbsim-gui/               # eframe app: optics, inputs, panels, audio (cpal)
    mbsim/                   # the binary: CLI, headless runner, scenario engine, record/replay
  assets/board/              # front/back artwork (SVG, own drawing; CC BY-SA 4.0 where schematic-derived)
  fixtures/                  # FICR/UICR dumps (unique IDs replaced), calibration data, golden traces
  firmware/                  # csolution test firmware: ISA tests, peripheral tests, calibration loops
  cmsis/                     # debug adapter template + registry snippet, example launch.json
  xtask/                     # SVD codegen, conformance runs vs FVP/hardware, packaging
  docs/                      # user guide, architecture notes, peripheral coverage
```

Crate choices (pin exact versions in `Cargo.lock`):

| Need | Crate | Note |
|:--|:--|:--|
| GDB RSP | `gdbstub` + own `Arch` for Cortex-M | Has target.xml override, memory map, monitor commands, flash (vFlash), breakpoints and watchpoints. Spike S2 checks multi-client and the packets the CMSIS Debugger sends. Fallback: own RSP layer (about 2k lines). |
| GUI | `eframe`/`egui` with the `wgpu` backend | Immediate mode, runs on all three OSs. Custom wgpu paint callback for the LED glow. |
| Artwork | `resvg`/`usvg` | Renders the board SVG to textures at the current DPI. |
| Audio | `cpal` | CoreAudio, WASAPI, ALSA (PipeWire and PulseAudio through ALSA); speaker output and host microphone input. |
| ELF/HEX | `object`, `ihex` | |
| YAML (cbuild-run, scenarios) | a maintained serde YAML crate | `serde_yaml` is archived; evaluate `serde_yaml_ng` or `saphyr`. |
| SVD codegen | `svd-parser` (in `xtask`) | Generates register tables (offset, reset value, access, write masks). |
| Disassembly | `yaxpeax-arm` | `monitor disassemble`, instruction traces, decoder cross-check. |
| Channels | `crossbeam-channel`, `rtrb` | Lock-free SPSC rings for pin log and audio. |
| CLI, logs | `clap`, `tracing` | |
| Tests | `proptest`, `egui_kittest` | Random instruction streams; GUI snapshot tests. |
| Release | `cargo-dist`, `cargo-about`, `cargo-zigbuild` | Installers, licence bundle, glibc baseline. |

## CPU core (`mbsim-cpu`)

Execution:

- A table-driven decoder turns 16- and 32-bit Thumb encodings into an `Op` enum.
- A predecode cache per 4 KB page of flash and code RAM holds decoded ops (later: decoded basic blocks). It is
  invalidated by NVMC writes and erases, by stores to RAM pages that hold decoded code, and on `ISB` after
  such writes.
- Memory fast path: a page table maps RAM and flash pages directly to slices. MMIO pages dispatch to device
  models.
- Interpreter loop without dynamic dispatch on the hot path. `unsafe` is allowed only in the memory fast path.

ISA coverage (all required for 1.0, because GCC `-mcpu=cortex-m4 -mfpu=fpv4-sp-d16 -mfloat-abi=hard`, newlib
and RTOS kernels use them):

- **ARMv7-M base**:
  - all Thumb-16 and Thumb-32 instructions, `IT` blocks;
  - `LDREX`/`STREX`/`CLREX` with a local exclusive monitor (cleared on exception entry and return);
  - `CBZ`/`CBNZ`, `TBB`/`TBH`, `SDIV`/`UDIV` (with the `CCR.DIV_0_TRP` trap);
  - `BKPT`, `SVC`, `UDF`, barriers, `WFI`/`WFE`/`SEV` with the event register, `CPS`, `MRS`/`MSR` for all
    special registers.
- **DSP extension**: SIMD add/sub/halving/saturating (`QADD8`…`USAX`), `SSAT`/`USAT`/`SSAT16`, packing
  (`PKHBT`, `SXTAB16`…), `SEL` with `APSR.GE`, `SMLA<x><y>`, `SMLAL*`, `SMUAD`/`SMUSD`, `SMMLA`/`SMMUL`, `UMAAL`,
  `USAD8`/`USADA8`.
- **FPv4-SP-D16**:
  - all single-precision data processing including `VFMA`/`VFNMA`, `VDIV`, `VSQRT`, `VCVT` (fixed and
    integer, R/round modes);
  - `VLDM`/`VSTM`/`VPUSH`/`VPOP`, `VMOV` variants, `VMRS`/`VMSR`;
  - FPSCR flags, rounding modes, flush-to-zero, default NaN, AHP; `CPACR` access checks (NOCP UsageFault).
- **Exception model**:
  - 48 external IRQs (0..47 per `nrf52833.h`, with gaps at 30, 31, 41..44, 46);
  - 3 priority bits, PRIGROUP, tail chaining, late arrival, `EXC_RETURN` forms;
  - `CCR.STKALIGN`, lazy FP stacking (`FPCCR.ASPEN/LSPEN`, `FPCAR`);
  - fault escalation to HardFault, lockup at `0xEFFFFFFE`, `SLEEPONEXIT`;
  - `AIRCR.SYSRESETREQ` causes a system reset through the device (`POWER.RESETREAS.SREQ`); `VTOR`.
- **System control space**:
  - `CPUID = 0x410FC241` (r0p1), ID registers, `MVFR0/1`;
  - `ICSR`, `SHCSR`, `CFSR`/`HFSR`/`DFSR`/`MMFAR`/`BFAR`;
  - NVIC `ISER`…`IPR` and `STIR`;
  - SysTick (calibration value captured from the board);
  - MPU with 8 regions (PMSAv7, background region, `MPU_TYPE = 0x800`).
- **Debug components**:
  - DCB `DHCSR` (`C_DEBUGEN`, `C_HALT`, `C_STEP`, `C_MASKINTS`, `S_RETIRE_ST`, `S_LOCKUP`, `S_SLEEP`), `DCRSR`,
    `DCRDR`;
  - `DEMCR` (`TRCENA`, vector catch `VC_CORERESET`/`VC_HARDERR`/…);
  - DWT: `CYCCNT`, `CPICNT`…`FOLDCNT`, `PCSR`, 4 comparators with `FUNCTION` and `MATCHED`;
  - FPB: 6 code + 2 literal comparators, revision 1;
  - ITM: stimulus ports routed to a console or TCP port;
  - TPIU and ROM table with CoreSight ID registers, so that pyOCD's discovery works in the probe mode.

Debug semantics follow the hardware:

- `BKPT` halts when `C_DEBUGEN` is set, raises DebugMonitor when `DEMCR.MON_EN` is set, and otherwise
  escalates to HardFault.
- FPB hits halt before the instruction executes.
- DWT watchpoints halt after the access instruction, as the Cortex-M4 does. The stop reply carries
  `watch:`/`rwatch:`/`awatch:` with the address.
- Stepping with `C_MASKINTS` honours the setting.

## nRF52833 device (`mbsim-nrf52833`)

Memory map:

| Range | Content | Model |
|:--|:--|:--|
| `0x0000_0000`–`0x0007_FFFF` | Flash 512 KB, 4 KB pages | NVMC semantics: reads 0xFF after erase, writes only clear bits, `CONFIG.WEN/EEN`, `READY`, `ERASEPAGE`, `ERASEALL`, timing per PS; persisted in the board-state directory |
| `0x0080_0000`–`0x0081_FFFF` | Code RAM alias of Data RAM | Same storage as `0x2000_0000` |
| `0x1000_0000` | FICR | Values dumped from the real board, unique IDs and keys replaced; `INFO.PART = 0x52833`, the errata ID words at `0x1000_0130/0134` |
| `0x1000_1000` | UICR | Persisted; `PSELRESET`, `NFCPINS`, `APPROTECT`, `REGOUT0`, customer words |
| `0x2000_0000`–`0x2001_FFFF` | Data RAM 128 KB | EasyDMA-capable |
| `0x4000_0000`–`0x4002_FFFF` | APB peripherals, 4 KB each; peripheral ID = `(addr >> 12) & 0x3F` = IRQ number | Shared task/event/shorts/`INTEN` framework |
| `0x5000_0000`, `0x5000_0300` | GPIO P0, P1 (AHB) | Access timing calibrated on the board |
| `0xE000_0000`– | Private peripheral bus | `mbsim-cpu` |
| Unmapped | | Precise BusFault for CPU accesses (reserved APB slots: verify on the board); GDB reads return an error instead of faulting |

Common device rules:

- Tasks trigger on writing 1.
- Events are set by the peripheral and cleared by writing 0. `INTEN` gates the IRQ line, and an event drives
  its IRQ until cleared.
- `SHORTS` act within the same tick.
- EasyDMA accepts only Data RAM. A pointer into flash transfers garbage on the chip; the model transfers zeros
  and logs a warning (the BSP knowledge unit warns about this case).
- Register metadata (offset, reset value, readable and writable masks) is generated from the SVD. The behaviour
  is hand-written per peripheral.
- Accesses to undocumented addresses written by `SystemInit` errata workarounds (`0x4000_0EE4`,
  `0x4000_5610..568C`, `0x4007_C074`, `0x4000_173C`, …) are accepted and logged at debug level.
- **Debugger accesses have no side effects**; CPU accesses do (for example the read of legacy `UART.RXD`).

Coverage:

| Peripheral | Used by | Behaviour to model | Phase |
|:--|:--|:--|:--|
| CLOCK | SystemInit, all timing | HFCLK/LFCLK start tasks and events, `HFCLKSTAT`, calibration events (errata 36), `TRACECONFIG` | 1 |
| POWER | SystemInit, reset | `RESETREAS` (pin, watchdog, SREQ, lockup, wake-ups), `SYSTEMOFF`, `DCDCEN`, `GPREGRET`, RAM retention registers | 1 |
| NVMC | SystemInit (UICR fixes), flash programming | See memory map, `ICACHECNF` (feeds the timing model) | 1 |
| FICR, UICR, APPROTECT | SystemInit | See memory map; `APPROTECT.DISABLE`/`FORCEPROTECT` registers | 1 |
| GPIO P0/P1 | every component | `OUT`/`OUTSET`/`OUTCLR`/`IN`/`DIR`/`DIRSET`/`DIRCLR`/`LATCH`/`DETECTMODE`; `PIN_CNF` with direction, input buffer connect, pull (13 kΩ typ), drive (S0S1, H0H1, S0D1, …), sense and DETECT; `IN` comes from the board net solver | 1 |
| TIMER0–4 | LED Matrix (TIMER1) | Timer and counter modes, prescaler, 8/16/24/32-bit, CC[4]/CC[6], `CAPTURE`, shorts, interrupts | 1 |
| UARTE0 | UART | EasyDMA `TXD`/`RXD` with `MAXCNT`/`AMOUNT`, `STARTRX`/`STOPRX`, `ENDRX_STARTRX` short, `RXSTARTED`, `RXTO`, `ERRORSRC`, exact frame time from the `BAUDRATE` value (0x01D60000 is 115108 baud) | 1 |
| TWIM0 | I2C, Motion Sensor | `STARTTX`/`STARTRX`/`STOP`, `LASTTX_STARTRX`/`LASTRX_STOP` shorts, `ERRORSRC` (address/data NACK, overrun), bit time per `FREQUENCY`, bus to the board's I2C devices | 1 |
| SAADC | Microphone | One-shot, scan and internal sample timer (`SAMPLERATE` MODE=Timers), gain 1/6…4, internal 0.6 V and VDD/4 references, `TACQ` + 2 µs conversion, oversampling, `RESULT` EasyDMA, limits, calibration task; input voltage from the board's analog nets | 1 |
| PWM0 | Speaker | Prescaler, up and up-down modes, `COUNTERTOP`, all four decoder modes, `SEQ[0..1]` with `REFRESH`/`ENDDELAY`, `LOOP`, shorts, polarity bit 15, pin output edges into the pin log | 1 |
| TEMP | Temperature | 36 µs conversion, `TEMP` in 0.25 °C from the board temperature input, the `A0..B5/T0..T4` registers that SystemInit copies from FICR | 1 |
| FPU (peripheral) | | IRQ 38 from FPSCR cumulative flags | 1 |
| RTC0–2, GPIOTE, PPI, EGU/SWI, RNG, WDT | not the BSP, but common in user code and RTOS kernels | Full | 2 |
| UARTE1, TWIM1, SPIM0–3, TWIS, SPIS, legacy UART/TWI/SPI, PWM1–3, QDEC, COMP, LPCOMP, PDM, I2S, MWU, ACL | edge connector use | Full | 2–3 |
| RADIO, NFCT, USBD, CCM, AAR, ECB | | Read-as-zero/write-ignored with a warning; `--strict` halts on access | — |

## Board model (`mbsim-microbit`)

Electrical model: each GPIO pin belongs to a net. A net's drivers are:

- the MCU pin, as configured by `PIN_CNF` (push-pull, open-drain, internal pull, input buffer connected or not);
- external resistors and parts from the schematic.

A small resistive solver (conductance-weighted node voltage) computes the net voltage on every change. `IN`
applies 0.3/0.7 × VDD thresholds; SAADC channels read the analog voltage. This reproduces the following
without special cases:

- buttons with external pull-ups;
- open-drain I2C with 1 kΩ pull-ups;
- the touch pad's RC charge;
- LEDs that do not light when a column pin is an input.

| Part | Nets (nRF52833) | Model | GUI |
|:--|:--|:--|:--|
| 5×5 LED matrix | ROW1..5 P0.21/22/15/24/19 (source), COL1..5 P0.28/11/31, P1.05, P0.30 (sink) | LED conducts when row voltage minus column voltage exceeds Vf (red LED, about 1.8 V). Current through the pin drive resistances (S0S1/H0H1) sets brightness. Every on/off edge goes to the pin log. | Front LEDs with glow |
| Buttons A/B | P0.14 / P0.23, 10 kΩ pull-ups R4/R5 | Switch to GND, optional contact bounce (off by default) | Click/hold, keys `A`, `B`, both |
| Reset button | nRESET P0.18 through the interface MCU | Pin reset (`RESETREAS.RESETPIN`); a long press later models the V2 power-off behaviour | Back side, key `R` |
| Touch logo | P1.04, R53 10 MΩ to VREG | RC charge, C = pad + finger. First estimate: pad about 2 pF, finger 50–500 pF; both are fitted to the measured counts (194 untouched, 4500–40000 touched). | Press on the logo; slider for finger capacitance |
| Speaker MLT-8530 | P0.00 → 4.7 kΩ → MOSFET T2 gate, 100 kΩ pull-down | Gate above threshold → coil on; audio from exact edge times with band-limited synthesis, optional transducer filter (resonance per datasheet) | Speaker on the back, audio out, volume/mute |
| Microphone SPU0410 | RUN_MIC P0.20 (power, high drive, 120 Ω/10 µF), MIC_IN P0.05/AIN3 | Unpowered: 0 V. Powered: supply rises with the RC, output = 75 mV bias (73 mV measured) + signal × sensitivity. Calibrate so that idle peak-to-peak ≈ 15–36 LSB and a clap ≈ 700 LSB at gain 4. | Host microphone (cpal), level slider, "clap" button, WAV file |
| Microphone LED D70 | RUN_MIC | Lit while the microphone is powered | Front, next to the logo |
| LSM303AGR | I2C P0.08/P0.16 at 0x19 and 0x1E, INT P0.25 (open-drain through transistors) | Register file of both functions; `WHO_AM_I` 0x33/0x40; sub-address auto-increment (bit 7, accelerometer); ODR timing, `STATUS` bits, BDU, FS/HR/LP scaling, FIFO (phase 2), magnetometer offset cancellation, temperature, INT1/DRDY. Physical mounting rotation of U6 relative to the PCB, calibrated in six orientations on the board. | Tilt and drag the board, pitch/roll sliders, shake/free-fall gestures, compass heading and field strength (default 0.5 gauss), hard-iron offset |
| Interface MCU (nRF52820 DAPLink) | UART P0.06 (TX) / P1.08 (RX); internal I2C; reset | UART ↔ host bridge with real frame timing. I2C at the interface address: NACK in phase 1, a minimal protocol in phase 2 (address 0x70 per CODAL, to verify). | Back side power and USB LEDs: power on while running, USB LED flickers on UART traffic and loads (colours to verify on the board) |
| Edge connector | P0..P20, 3V, GND | Pin levels exposed; phase 3: plug-in trait for external I2C/SPI/GPIO parts | Pin inspector overlay |
| Die temperature | — | Board temperature input (default 21.00 °C) | Slider |

Default sensor state: lying flat, face up, in a 0.5 gauss field. The BSP's validation values (acceleration
magnitude 1.02 g, field 0.5–0.75 gauss, 21 °C, logo 194/194, microphone bias 73 mV) are the reference for
defaults and tests.

## GDB server and CMSIS-Debugger integration (`mbsim-gdb`)

### RSP feature set

- `qSupported` with `PacketSize`, `qXfer:features:read+`, `qXfer:memory-map:read+`, `qXfer:threads:read+`,
  `QStartNoAckMode+`, `vContSupported+`, `swbreak+`, `hwbreak+`.
- Target description:
  - `org.gnu.gdb.arm.m-profile` (r0–r12, sp, lr, pc, xpsr);
  - `org.gnu.gdb.arm.vfp` (s0–s31/d0–d15, fpscr);
  - `org.gnu.gdb.arm.m-system` (msp, psp, primask, basepri, faultmask, control).
- Memory map: flash with `blocksize` 4096 (enables GDB `load` through `vFlashErase`/`vFlashWrite`/`vFlashDone`
  with NVMC semantics), RAM, code RAM alias, FICR/UICR, peripheral ranges.
- Packets:
  - `?`, `g`/`G`, `p`/`P`, `m`/`M`/`X`;
  - `c`/`s`/`C`/`S` and `vCont;c;s;t` (the FVP template turns `vCont` off, so both forms are needed);
  - `Z0`/`Z1` (software breakpoints in RAM; in flash they map to FPB comparators, as pyOCD does);
  - `Z2`–`Z4` (DWT comparators);
  - Ctrl-C, `D`, `k`, `qAttached`, `qC`, `qfThreadInfo`, `qRcmd`, `QStartNoAckMode`.
- **Multiple clients on one port**, as pyOCD 0.45.1 does:
  - The first connection is the controller.
  - Later connections are auxiliary. The cdt-gdb-adapter's `auxiliaryGdb` connects with
    `set remote interrupt-on-connect off`.
  - An auxiliary client gets a stop reply for `?` without halting the core. Its memory and register reads are
    served between quanta while the core runs. Run control from an auxiliary client is refused.
- `monitor` commands:
  - the subset of pyOCD's that the CMSIS templates and users send: `reset [halt|run|hw|sw]`, `halt`, `go`,
    `help`;
  - simulator commands: `sim speed <x|max|realtime>`, `sim time`, `sim button a press|release`,
    `sim tilt <pitch> <roll>`, `sim touch on|off`, `sim temp <C>`, `sim mic <level>`, `sim uart <text>`,
    `sim trace on|off`, `sim snapshot save|load`, `sim stats`.
- **CPU Time**: the CMSIS Debugger enables its CPU Time view only when `DWT_CTRL` (0xE000_1000) shows
  `CYCCNTENA = 1` and `NOCYCCNT = 0`, and only for configurations with a `cmsis` node. On debugger connect the
  simulator sets `DEMCR.TRCENA` and `DWT_CTRL.CYCCNTENA`, as a probe-based server does (configurable).
  `CYCCNT` counts core cycles.
- Start state:
  - With a GDB port, the core stays halted at the reset vector until the first resume, as an FVP started
    with its GDB plugin.
  - Without one (Load & Run), the core runs immediately.
  - `--persist` keeps the process and the board alive across debug sessions, like a board on the desk.

### Ways to use it from VS Code

Implement the modes in this order:

**Mode A — `Arm-FVP` debug adapter, no change to Arm tools.** The FVP template
(`templates/debug/FVP.adapter.json`) starts `config.model` with `-D --plugin …/GDBServer.so -C
GDBServer.port=<port> [-f config] [args] -a <image>` and connects GDB to the port. `mbsim` accepts this FVP
command-line subset:

- `-C GDBServer.port=N` sets the port;
- `-a [core=]file` loads the image;
- `-f` reads a parameter file;
- `-C board.*=…` sets simulator parameters;
- `--plugin` and `-D` are ignored, as are unknown FVP options (with a warning).

```yaml
target-types:
  - type: microbit-v2-sim
    board: MicroBit::microbit-v2
    target-set:
      - set:
        debugger:
          name: Arm-FVP
          model: mbsim                  # on PATH, or an absolute path
          args: --uart-tcp 4444
        images:
          - project-context: BoardTest.Debug
```

Limitations:

- The FVP template sets `auxiliaryGdb: false`, so Live Watch and the views that read while running stay off.
- The "CMSIS Load" task only echoes; the image comes from `-a`.

Spike S1 verifies that csolution and CMSIS Solution accept a `model` outside the enum and that a missing
`AVH_FVP_PLUGINS` does no harm.

**Mode B — own debug adapter `microbit-sim`.** The project ships:

- `cmsis/microbit-sim.adapter.json`: a copy of `CMSIS-DAP-pyOCD.adapter.json` with `server: mbsim`,
  `auxiliaryGdb: true`, the telnet section mapped to the UART TCP port, and `CMSIS Load` = `mbsim load
  --cbuild-run …`, which programs the running board or the board-state directory;
- a `debug-adapters.yml` entry.

`mbsim register-cmsis-adapter [--apply --backup-dir …]` installs both into:

- `<extension>/templates/debug/`;
- `<extension>/tools/cmsis-toolbox/etc/`;
- each listed toolbox's `etc/`.

It follows dry-run/backup/collision rules: nothing changes without `--apply`, every changed file is backed up, and a foreign file of the same name is never overwritten.
For pyOCD-like commands, `mbsim gdbserver` accepts pyOCD's flags (`--cbuild-run`, `--port`, `--persist`,
`--reset-run`, `--quiet`, `--log`), so the template differs from the pyOCD one only in the server name.

Long term, propose a generic simulator adapter to Open-CMSIS-Pack, so no installer is needed.

**Mode C — hand-written `launch.json`** (`updateConfiguration: manual`). This always works and is the
documented fallback:

```json
{
  "name": "micro:bit V2 simulator (launch)",
  "type": "gdbtarget",
  "request": "launch",
  "cwd": "${workspaceFolder}/Examples/BoardTest",
  "program": "out/BoardTest/microbit-v2/Debug/BoardTest.elf",
  "gdb": "arm-none-eabi-gdb",
  "auxiliaryGdb": true,
  "initCommands": ["monitor reset halt", "tbreak main"],
  "customResetCommands": ["monitor reset halt", "maintenance flush register-cache",
                          "maintenance flush dcache", "tbreak main", "continue"],
  "target": {
    "server": "mbsim",
    "serverParameters": ["gdbserver", "--cbuild-run", "${command:cmsis-csolution.getCbuildRunFile}",
                         "--port", "3333", "--uart-tcp", "4444"],
    "port": "3333"
  },
  "cmsis": { "cbuildRunFile": "${command:cmsis-csolution.getCbuildRunFile}", "updateConfiguration": "manual" }
}
```

The `cmsis` node must stay: the CMSIS Debugger uses it for CPU Time and to find the SVD for the Peripheral
Inspector.

**Mode D (after 1.0) — pyOCD remote probe.** Implement pyOCD's TCP debug-probe protocol
(`pyocd/probe/tcp_probe_server.py` / `tcp_client_probe.py` in the pyOCD 0.45.1 the CMSIS Debugger bundles)
on top of a simulated SWD-DP, AHB-AP and the nRF52833 CTRL-AP. Then real pyOCD connects with
`--probe remote:localhost:5555`:

- it flashes with the pack's `nrf52xxx.flm`, which executes inside the simulated core;
- its own gdbserver, semihosting and RTT work unchanged;
- APPROTECT and `ERASEALL` behave as on the chip.

This gives the highest fidelity of the debug path, but it is slower and needs its own `launch.json`.

### Other interfaces

- **UART**: raw TCP server (default port 4444, the pyOCD telnet convention) for the VS Code Serial Monitor in
  TCP mode; a built-in terminal in the GUI; optional pseudo-terminal on macOS and Linux (`--uart-pty`, prints
  the device path), which serial-port tools need. Windows has no virtual COM port without a third-party
  driver; TCP is the supported path there.
- **Control port** (localhost, JSON lines): `load`, `reset`, input events, state queries. Used by `mbsim load`,
  scenario runners and future tooling.
- **Semihosting** (`BKPT 0xAB`): `SYS_WRITE0`/`WRITEC`/`WRITE`/`READC`/`EXIT`/`CLOCK`/`TIME` to the console or
  TCP, so test firmware can print and exit with a code.
- **CMSIS Developer Assistant**: with modes A–C the assistant's debug tools reach the simulator through the
  CMSIS Debugger session, so `pause_execution`, `read_peripheral_register`, `read_cycle_counter` and
  `get_fault_info` work without hardware. Its `serial_*` tools need a serial device: use `--uart-pty`, and
  check whether they accept TCP.

## GUI (`mbsim-gui`) — the optics of the board

Technology:

- eframe/egui on wgpu (Metal, DX12/Vulkan, Vulkan/GL), so one code base runs on all three OSs.
- Board artwork as SVG at 1:1 millimetre scale (the board is 51.6 × 42 mm), drawn for this project from the
  V2.2.1 schematic and the published dimensions. It is not a copy of official artwork (see risks). It is
  rendered with resvg at the display's DPI.
- LED glow: an additive bloom pass in a wgpu paint callback; pre-rendered glow sprites as the fallback on GL.

Views:

- **Front**:
  - 25 LEDs in their real 5 × 5 positions;
  - buttons A and B;
  - the gold touch logo;
  - the microphone hole and its LED;
  - the edge connector with rings 0, 1, 2, 3V, GND and the 20 small pads;
  - board revision label.
- **Back**:
  - nRF52833, LSM303AGR, the speaker;
  - USB micro-B, battery JST;
  - reset button, power LED, USB LED.
- A flip control switches between front and back (animated). A side-by-side layout is an option.
- Phase 2: a 3D view (wgpu) that tilts with the accelerometer input.

LED rendering:

- **Exact duty cycle, not sampling.** The display multiplexes one row per millisecond (200 Hz frames). Sampling
  pins at 60 Hz would alias. Instead, for every displayed frame the GUI integrates each LED's on-time from the
  pin-change log over a persistence window (default 20 ms).
- Perceived brightness: luminance ∝ average current, then sRGB transfer, then a brightness calibration fitted
  to photos of the real board at fixed exposure. The same fit sets the off-state colour (the LED body).
- **Eye mode** (default) shows the time-averaged image. **Camera mode** shows instantaneous states in slow
  motion, which makes multiplexing and ghosting bugs visible.
- The microphone LED, power LED and USB LED use the same pipeline.

Inputs:

| Input | Mouse / touch | Keyboard | Panel |
|:--|:--|:--|:--|
| Buttons A, B | press and hold | `A`, `B`, `Space` = both | — |
| Touch logo | press and hold on the logo | `T` | finger capacitance |
| Reset | back side button | `R` | — |
| Orientation | drag the board to tilt | arrow keys | pitch/roll/yaw, shake, free fall |
| Compass | — | — | heading, field strength, inclination |
| Sound | — | `C` = clap | host microphone on/off, level, WAV file |
| Temperature | — | — | °C |
| Program | drop a `.hex`/`.elf` on the board (programs flash and resets, like the MICROBIT drive) | — | — |

Panels (dockable):

- UART terminal: local echo off; CR/LF handling as in VS Code's Serial Monitor.
- Pin inspector: the edge connector overlay shows each pin's configuration and level; hover for the net.
- Peripheral summary.
- Sim control: run/pause, speed 0.01×…max, sim time, MIPS, real-time factor, record/replay.
- Log: warnings such as EasyDMA from flash or unimplemented register accesses.
- Audio: volume, mute, transducer filter.

The GUI shows the board even when no image is loaded (a factory board with erased flash), and keeps the window
across debug sessions with `--persist`.

## Timing and performance

- **Cycle model**:
  - Cortex-M4 TRM cycle counts: pipeline refill 1–3 on branches, `LDM`/`STM` N+1, `SDIV`/`UDIV` 2–12 depending
    on the data, `VDIV`/`VSQRT` 14, load/store pipelining.
  - A flash wait-state and `NVMC.ICACHECNF` cache model whose parameters are fitted to `DWT->CYCCNT`
    measurements of calibration loops on the board.
  - The BSP's `microbit_delay_us` busy loop and the touch charge-time loop are part of the calibration set.
- **Peripheral timing**: exact in their own clocks (see Architecture, time).
- **Speed techniques**: predecoded ops and later basic blocks; direct-mapped memory pages; quanta up to the
  next event; WFI skip; no allocation on the hot path; PGO for release builds.
- **Benchmarks** in `firmware/bench` (CoreMark, Dhrystone, BoardTest idle) run in CI and report MIPS and the
  real-time factor (non-blocking).

## Headless mode, scenarios, record and replay

```
mbsim run --headless --image BoardTest.elf --scenario tests/boardtest.yaml --max-time 10s
```

A scenario lists timed inputs and assertions:

```yaml
- at: 0ms
  expect-uart: "BBC micro:bit V2 BoardTest (nRF52833, SystemCoreClock 64000000 Hz)"
- at: 1500ms
  press: A
- at: 1600ms
  expect-leds: ["01110", "10001", "11111", "10001", "10001"]   # 'A' glyph
- at: 2000ms
  release: A
- at: 2100ms
  uart-send: "t"
- at: 2150ms
  expect-pin: { P0.00: toggling, freq-hz: 880, tolerance: 1% }
```

- Exit code 0 when all expectations pass. A JUnit XML report is optional.
- `--record session.mbrec` stores the image hash, board state and input log; `--replay` reproduces the run.
- Snapshots (`sim snapshot save/load`) serialise the full machine state (phase 2).

## Verification strategy

1. **ISA, unit level**: tests per instruction, written from the ARMv7-M pseudocode, including flags and
   corner cases (shifts by 32, `SSAT` boundaries, FP NaN, denormal and rounding cases).
2. **ISA, differential**: generate random instruction streams with `proptest` (integer, DSP, FP, IT blocks,
   load/store into a sandbox region) and run them in lockstep against Arm FVP_MPS2_Cortex-M4. FVPs come with
   the Arm Tools Environment Manager and can be driven over Iris or RSP. Compare
   the register file and touched memory after each instruction. Shrink failures automatically. Run a smaller
   corpus on the real nRF52833 through the CMSIS Debugger for the behaviour where the FVP and the silicon
   could differ.
3. **Exception and system level**: CMSIS 6 CoreValidation for Cortex-M4F; RTX5 and FreeRTOS demos (SVC,
   PendSV, lazy FP stacking, SysTick); fault tests (unaligned, divide by zero, MPU violation, bus fault,
   escalation, lockup).
4. **Peripherals**:
   - register tests generated from the SVD (reset values, RO/WO masks);
   - behaviour tests from the PS (one `firmware/periph/*` test image per peripheral, semihosting exit code);
   - golden captures from the real board taken with the CMSIS Developer Assistant tools: registers at
     breakpoints, event timing with `DWT->CYCCNT`, serial output.
5. **Board**:
   - BoardTest and Blinky scenario tests (R1);
   - LED patterns for every glyph of the BSP font;
   - speaker frequency at 31 Hz, 440 Hz, 880 Hz and 20 kHz;
   - microphone levels; accelerometer readings in six orientations against the hardware capture.
6. **Debugger**:
   - batch scripts for `arm-none-eabi-gdb` over RSP (breakpoints, watchpoints, `load`, reset, aux client
     while running);
   - comparison of RSP packet traces (`set debug remote 1`) between a CMSIS Debugger session on pyOCD with the
     real board and one on the simulator;
   - a manual checklist per release and OS: Load & Debug, attach, Live Watch, CPU Time, Peripheral Inspector,
     Core Peripherals, Serial Monitor, Reset Target.
7. **GUI**: `egui_kittest` snapshot tests of front/back views and LED states; manual visual check against
   photos of the board.

## Cross-platform build and distribution

| Target | Runner (GitHub Actions) | Notes |
|:--|:--|:--|
| `x86_64-pc-windows-msvc` | `windows-latest` | MSI and ZIP, Authenticode signed |
| `aarch64-pc-windows-msvc` | `windows-11-arm` | |
| `aarch64-apple-darwin` | `macos-latest` (arm64) | `.app` and CLI, signed and notarised (the CLI is started by VS Code tasks, so it must be notarised too) |
| `x86_64-apple-darwin` | cross-compiled on arm64, tested under Rosetta | Universal2 binary via `lipo` |
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` | `cargo-zigbuild` for a glibc 2.31 baseline; `.tar.gz`, `.deb`, AppImage |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` | |

- Cargo features: `gui` (default), `audio` (default). The headless build without them has no GPU or audio
  libraries, for CI containers.
- Every target runs headless tests on each push. GUI snapshot tests run on all three OSs with software
  rendering where no GPU exists (WARP, llvmpipe).
- `cargo-dist` builds installers and a shell/PowerShell installer. `cargo-about` produces the licence bundle,
  including Nordic's BSD-3-Clause for the vendored SVD and the CC BY-SA 4.0 notice for schematic-derived
  artwork.
- Optional: publish `mbsim` as a vcpkg artifact, so `vcpkg-configuration.json` (Arm Tools Environment Manager)
  can pin it like an FVP.
- Licence of the project itself: Apache-2.0, like the BSP (to confirm).

## Milestones

Rough sizes in person-weeks for one engineer experienced in Rust and Cortex-M. About 25 weeks to 0.1.

| # | Milestone | Content | Exit criterion | Size |
|:--|:--|:--|:--|:--|
| M0 | Spikes | **S1** Mode A: CMSIS Solution generates a working FVP launch for `model: mbsim` with a stub server. **S2** `gdbstub` with two clients against cdt-gdb-adapter 2.9.1 (`auxiliaryGdb: true`), `qRcmd`, memory map, vFlash. **S3** egui + wgpu bloom of 25 LEDs at 60 fps on all OSs. **S4** interpreter prototype of 30 instructions, MIPS measured. **S5** capture reference data from the board (see first steps). | Go/no-go per spike written down in `docs/decisions/` | 1.5 |
| M1 | Core and minimal device | Thumb-2 integer ISA, exceptions, NVIC, SysTick, SCB; GPIO, CLOCK, POWER, NVMC, FICR/UICR; ELF/HEX/cbuild-run loader; headless runner | `Blinky.elf` runs headless; scenario asserts LED row 1 blinking at 1 Hz and buttons mirrored | 4 |
| M2 | GDB server | RSP, target.xml, memory map, breakpoints (FPB + software), watchpoints, monitor commands, multi-client, modes A and C | CMSIS Debugger: Load & Debug of Blinky stops at `main`; step, breakpoints, Reset Target, Live Watch and CPU Time work on macOS | 2.5 |
| M3 | Complete core | DSP, FPv4-SP, MPU, fault model, lazy stacking, DWT/ITM; lockstep harness against the FVP | Random differential corpus (10⁷ instructions) without divergence; CoreValidation passes | 4 |
| M4 | BSP peripherals and board | TIMER, UARTE + TCP bridge, TWIM + LSM303AGR, SAADC + microphone, PWM + speaker, TEMP, touch RC, net solver | `BoardTest.elf` scenario passes (R1), every key command works | 4 |
| M5 | GUI optics | Artwork front/back, LED integrator and bloom, all inputs, panels, audio, drag-and-drop | R3 checklist; snapshot tests green on three OSs | 3.5 |
| M6 | Calibration and speed | Timing model fitted to the board; touch, delay and microphone numbers within tolerance; performance work | R6 met; calibration report in `docs/` | 2.5 |
| M7 | Release 0.1 | Packaging, signing, notarisation, mode B adapter installer, user guide, BSP example with a `microbit-v2-sim` target type | Signed builds for all six targets; manual debugger checklist passed on Windows, macOS, Linux | 2.5 |
| M8 | After 0.1 | Phase-2 peripherals (RTC, GPIOTE, PPI, …), mode D (pyOCD remote probe), 3D view, snapshots, edge-connector plug-ins, interface-MCU I2C protocol, LSM303AGR FIFO | Per feature | — |

## Risks and open questions

| Risk | Impact | Mitigation |
|:--|:--|:--|
| CMSIS Solution rejects a non-FVP `model` or needs `AVH_FVP_PLUGINS` | Mode A fails | Spike S1; modes B and C don't depend on it |
| `gdbstub` cannot serve the auxiliary client or the CMSIS Debugger's packet mix | Live Watch, CPU Time | Spike S2; own RSP layer as fallback |
| Undocumented silicon behaviour (errata registers, EasyDMA from flash, LSM303AGR details, interface MCU) | Firmware behaves differently than on the board | Capture from hardware; `--strict` mode reports every access the model does not cover |
| Timing-dependent code (touch counts, busy delays) depends on flash wait states and cache | R1 tolerances | Calibration set in M6; tolerances stated in the docs |
| Real time on slow machines | Audio dropouts, slow LEDs | WFI skip, quanta, basic-block cache; audio follows sim time with a jitter buffer and mutes when the sim falls behind |
| Serial tools expect a serial device | Tools cannot open the UART | TCP plus PTY; document the Windows limitation |
| "micro:bit" is a trademark of the Micro:bit Educational Foundation; the schematic is CC BY-SA 4.0 | Public release | Own artwork; check the Foundation's brand guidelines before publishing name and logo shape; keep schematic-derived assets under CC BY-SA with attribution |
| macOS notarisation and Windows signing of a server started by VS Code | Gatekeeper/SmartScreen blocks it | Sign in CI from M7; document unsigned dev builds |

Open decisions for the project owner:

1. ~~Project and binary name for a public release~~ — `mbsim`, see
   [decision 0008](docs/decisions/0008-name-and-trademarks.md).
2. Licence: Apache-2.0 like the BSP?
3. Priority of mode D (pyOCD remote probe) against phase-2 peripherals after 0.1.
4. Minimum host for the real-time requirement R6.

## First steps

1. Create the Cargo workspace skeleton with the crates above, CI for the six targets, `cargo deny` and
   `cargo-about`.
2. Run spikes S1–S4; record each decision in `docs/decisions/`.
3. Spike S5: capture reference data from the micro:bit with the CMSIS Developer Assistant tools (pyOCD session
   on the board):
   - FICR and UICR dumps;
   - `CPUID`, SysTick `CALIB`, `DWT_CTRL`, `FP_CTRL`;
   - ROM table and CoreSight IDs;
   - register values at breakpoints in BoardTest;
   - `DWT->CYCCNT` of calibration loops;
   - touch counts; accelerometer readings in six orientations;
   - photos of the lit LED matrix at fixed exposure; a recording of the 440 Hz and 880 Hz tones.
4. Vendor `nrf52833.svd` (BSD-3-Clause notice) and write the `xtask` SVD code generator.
5. Start M1 with `Blinky.elf` from the BSP as the first target image.
