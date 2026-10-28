# 0008 — Name `mbsim`, no third-party marks in names, a personal project

**Status:** accepted

## Context

The project started as `microbit-v2-sim` under the GitHub account `MatthiasHertelArm`. The Micro:bit
Educational Foundation's brand guidelines do not allow its marks in product, company or domain names; text
may say that something "works with the BBC micro:bit", and BBC elements may only appear as the plain text
"BBC micro:bit". Arm's trademark guidelines allow Arm, Cortex and Keil in accurate text, with ® at the
first prominent use and an ownership notice, but not inside other names. The author works for Arm, so the
project must not look like an Arm product.

## Decision

- The project, repository and CMSIS debug adapter are named `mbsim`, like the binary. The repository moves
  to `MatthiasHertel80/mbsim`.
- Text describes mbsim as a simulator that "works with the BBC micro:bit V2". Window title, `mbsim info`,
  monitor help and crate descriptions say `mbsim`. The board label in the window names the simulated board.
- The README carries the trademark notices and says that mbsim is a personal project, not an Arm product,
  and not affiliated with or endorsed by Arm, the Micro:bit Educational Foundation, the BBC or Nordic
  Semiconductor.

## Consequences

- `debugger: name: microbit-v2-sim` keeps working: the new catalog entry has it as an alias, and
  `mbsim register-cmsis-adapter` leaves an entry of 0.3 or earlier in place (it already answers to `mbsim`).
- Old GitHub URLs redirect to the new repository.
- The crate `mbsim-microbit` keeps its name: it is internal and names the board it models.
- The board artwork shows the micro:bit logo where the board has its touch logo; the Foundation can be asked
  about it, and it is redrawn if they object.
