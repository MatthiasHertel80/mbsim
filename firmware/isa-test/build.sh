#!/bin/sh
# Builds the ISA differential test (Arm GNU Toolchain on PATH or ARM_GCC_BIN).
set -e
cd "$(dirname "$0")"
GCC="${ARM_GCC_BIN:+$ARM_GCC_BIN/}arm-none-eabi-gcc"
mkdir -p out
"$GCC" -mcpu=cortex-m4 -mthumb -mfpu=fpv4-sp-d16 -mfloat-abi=hard -O1 -g -ffreestanding -nostdlib \
  -Wall -Wno-unused-variable -T link.ld startup.c isa_test.c -lgcc -o out/isa_test.elf
echo "out/isa_test.elf"
