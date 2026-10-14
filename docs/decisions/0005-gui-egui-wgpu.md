# 0005 — eframe/egui with wgpu; LED glow from additive sprites

**Status:** accepted

## Context

The GUI must show what the eye sees: the LED matrix is multiplexed row by row, so each LED is lit for a
fifth of the time or less, and brightness levels come from the duty cycle. It has to run on Windows,
macOS and Linux from one code base, next to a side panel with inputs and a UART terminal.

## Decision

- `eframe`/`egui` 0.36 with its default `wgpu` renderer (Metal, DirectX 12, Vulkan or GL).
- The board is drawn as vector artwork in millimetres from the board outline; no photographs.
- `Optics` integrates the on-time of every LED from the pin events of the simulation with a time constant
  (default 12 ms, adjustable as "persistence"); a "camera" view shows the instantaneous state.
- Glow is a precomputed radial texture drawn with additive blending, scaled by luminance. No bloom
  post-process, so no custom shader per back end.
- Audio uses `cpal` and follows simulated time; it is a cargo feature so a build without system audio
  libraries is possible.

## Consequences

- One renderer path on every OS; screenshots through `ViewportCommand::Screenshot` for documentation and
  tests (`MBSIM_SCREENSHOT`).
- The glow is an approximation of the real diffusion; a calibration against photos of the board at fixed
  exposure is still open.
