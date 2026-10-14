# Decisions

One file per decision: the context, what was decided, and what follows from it. A decision that is
replaced keeps its file and links to the one that replaces it.

| # | Decision |
|:--|:--|
| [0001](0001-own-rsp-server.md) | Own GDB remote serial protocol server instead of `gdbstub` |
| [0002](0002-interpreter-with-decode-cache.md) | Interpreter with a decode cache, no JIT |
| [0003](0003-time-base-and-scheduler.md) | 512 MHz tick time base and an event scheduler |
| [0004](0004-cmsis-debugger-integration.md) | Three ways into the CMSIS Debugger; the Arm-FVP form first |
| [0005](0005-gui-egui-wgpu.md) | eframe/egui with wgpu; LED glow from additive sprites |
| [0006](0006-reference-fvp.md) | The Arm FVP as the reference for the core, recorded once |
| [0007](0007-hand-written-peripherals.md) | Hand-written peripheral models, no SVD code generator |
