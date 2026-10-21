# Getting started

mbsim runs firmware for the BBC micro:bit V2 on your computer, in a window that shows the board. There are
three ways to use it:

- [Standalone with your own firmware](#standalone-with-your-own-firmware): an `.elf`, `.hex` or `.bin`
  built with any toolchain.
- [Standalone with MicroPython](#standalone-with-micropython): the REPL and your Python programs, no board
  needed.
- [In Keil Studio for VS Code](#in-keil-studio-for-vs-code): build a CMSIS solution and run or debug it in
  the simulator.

The [README](../README.md) is the reference for all options; `mbsim --help` lists them too.

## Install

Download the archive for your computer from the
[releases](https://github.com/MatthiasHertelArm/microbit-v2-sim/releases), unpack it and put `mbsim`
(`mbsim.exe` on Windows) on your `PATH`.

- macOS: the binary is not signed. Clear the quarantine attribute once:
  `xattr -d com.apple.quarantine mbsim`.
- Linux: the window needs X11 or Wayland with libxkbcommon, and sound needs libasound2.

## Standalone with your own firmware

```sh
mbsim app.elf
```

Any image for the nRF52833 that runs on a micro:bit V2 works: an ELF (which also gives a debugger the
symbols), an Intel HEX file or a raw binary for address 0.

In the window:

- **A**, **B** and **T** (or a mouse click) press the buttons and touch the logo, **Space** presses both
  buttons, **R** resets and **F** shows the back of the board.
- Right-drag tilts the board. The side panel shakes it and sets heading, temperature and the sound at the
  microphone.
- The UART panel is the USB serial port: it shows what the firmware prints and sends a line you type.
- To load a new build, drop the `.elf` or `.hex` on the window, or program the running simulator from a
  terminal:

  ```sh
  mbsim load app.elf
  ```

Without the window, or with a terminal program on the serial port:

```sh
mbsim app.elf --headless --time 5    # 5 s of simulated time, UART on stdout
mbsim app.elf --uart-tcp 4444        # UART on TCP port 4444
```

To debug with any GDB for Arm, start the simulator as a GDB server. The core waits halted until the
debugger connects:

```sh
mbsim app.elf --gdb-port 3333
arm-none-eabi-gdb app.elf -ex "target remote :3333"
```

For automated tests, a [scenario](../README.md#scenarios) presses buttons, tilts the board and checks the
LEDs, tones and UART output in simulated time.

**Not supported yet:** MakeCode and the micro:bit Python Editor save *Universal Hex* files, which hold a V1
and a V2 image in one file. mbsim does not read that format and stops with
`HEX: unsupported IHEX record type '0A'`.

## Standalone with MicroPython

1. Download MicroPython for the micro:bit V2, `micropython-microbit-v2.1.2.hex`, from its
   [release page](https://github.com/microbit-foundation/micropython-microbit-v2/releases/tag/v2.1.2).
2. Start it:

   ```sh
   mbsim micropython-microbit-v2.1.2.hex
   ```

3. The UART panel shows the MicroPython REPL. Type a line in "send to the target" and press Enter:

   ```python
   from microbit import *
   display.show(Image.HEART)
   ```

   The heart appears on the LED matrix, as in the picture in the [README](../README.md).

To run a whole program, send it in MicroPython's paste mode: Ctrl+E starts paste mode and Ctrl+D runs what
was pasted. From a POSIX shell (macOS, Linux, Git Bash on Windows):

```sh
{ sleep 2; printf '\005'; cat main.py; printf '\004'; } |
    mbsim micropython-microbit-v2.1.2.hex --uart-stdout --uart-stdin
```

The window opens, and after two seconds, when MicroPython has started, the program runs. Its output appears
in the terminal and in the UART panel. The program is not stored on the board, so a reset removes it. With
`--headless --time 10` in place of `--uart-stdout`, the same program runs without the window for 10 s of
simulated time.

The board's microphone can hear your computer's microphone: tick **host microphone** in the side panel, or
start with `--mic`. Then `microphone.sound_level()` follows your voice.

## In Keil Studio for VS Code

Keil Studio for VS Code is the Arm Keil Studio Pack of extensions, which includes CMSIS Solution and the
CMSIS Debugger. mbsim needs no extension of its own: CMSIS Solution starts it like an Arm FVP.

1. **Get a project for the board.** The board support pack `MicroBit::MicroBit_V2_BSP` describes the board
   and has two examples, Blinky and BoardTest. Install the pack from a VS Code terminal, where Keil Studio
   puts the CMSIS-Toolbox on the `PATH`:

   ```sh
   cpackget add https://github.com/MatthiasHertelArm/MicroBit_V2_BSP/releases/download/v0.1.1/MicroBit.MicroBit_V2_BSP.0.1.1.pack
   ```

   Then copy `Examples/BoardTest` from the pack folder into your workspace. The pack folder is
   `~/.cache/arm/packs/MicroBit/MicroBit_V2_BSP/0.1.1` on macOS and Linux, and
   `%LOCALAPPDATA%\Arm\Packs\MicroBit\MicroBit_V2_BSP\0.1.1` on Windows. Your own solution works too if it
   uses `board: MicroBit::microbit-v2`.

2. **Add a target type for the simulator** to `target-types:` in the `*.csolution.yml`, next to the one for
   the real board:

   ```yaml
       - type: microbit-v2-sim
         board: MicroBit::microbit-v2
         target-set:
           - set:
             debugger:
               name: Arm-FVP
               model: /path/to/mbsim          # C:\path\to\mbsim.exe on Windows
               args: -C sim.uart_port=4444
             images:
               - project-context: BoardTest.Debug
   ```

3. **Build:** in the CMSIS view, select the target type `microbit-v2-sim` and build.

4. **Run or debug:**
   - **Run** starts the simulator with the image, and the board window opens.
   - **Debug** starts it as a GDB server, loads the image and stops at `main`. Breakpoints, watchpoints
     and stepping work as on the real board.

5. **Serial output:** open the Serial Monitor (View › Serial Monitor), choose TCP and connect to
   `localhost`, port 4444.

The `args:` line takes further options as FVP parameters (`-C sim.<name>=<value>`):

| Parameter | Effect |
|:--|:--|
| `sim.disable-visualisation=1` | no window |
| `sim.out_file=-` | UART output in the VS Code terminal |
| `sim.speed=max` | as fast as possible (`1` is real time) |
| `sim.mic=1` | the host microphone feeds the board's microphone |
| `sim.persist=1` | keep running when the debugger disconnects |

**Live Watch** in the CMSIS Debugger needs a second connection to the simulator, which the Arm-FVP adapter
switches off. mbsim brings its own adapter for this. Install it into CMSIS Solution:

```sh
mbsim register-cmsis-adapter                                  # shows what it would change
mbsim register-cmsis-adapter --apply --backup-dir ~/mbsim-backup
```

Then use `debugger: name: microbit-v2-sim` in place of `name: Arm-FVP` (no `model:` line; `mbsim` must be
on the `PATH`). The adapter lives in the extension's folder, so register it again after CMSIS Solution
updates.

**MicroPython** is not a CMSIS solution, so it runs [standalone](#standalone-with-micropython). To have its
REPL inside VS Code, start it with `--uart-tcp 4444` and connect the Serial Monitor to port 4444 as above.
