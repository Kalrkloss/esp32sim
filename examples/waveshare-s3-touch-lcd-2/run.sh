#!/bin/sh
# Run your own ESP32-S3 image on the emulated Waveshare ESP32-S3-Touch-LCD-2 (ST7789 240x320,
# CST816D touch, 16 MB flash, 8 MB PSRAM).
#   LCD2_BUILD=/path/to/build LCD2_APP=app.bin examples/waveshare-s3-touch-lcd-2/run.sh [flags...]
# LCD2_BOOTLOADER and LCD2_PTABLE override the bootloader and partition table inside the build;
# LCD2_ELF adds symbols for console backtraces.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
B=${LCD2_BUILD:?set LCD2_BUILD to an ESP-IDF build directory}
APP=${LCD2_APP:?set LCD2_APP to the application .bin inside $B}
BL=${LCD2_BOOTLOADER:-$B/bootloader/bootloader.bin}
PT=${LCD2_PTABLE:-$B/partition_table/partition-table.bin}
[ -f "$B/$APP" ] || { echo "no $B/$APP: set LCD2_APP" >&2; exit 2; }
[ -f "$BL" ] || { echo "no $BL: set LCD2_BOOTLOADER" >&2; exit 2; }
[ -f "$PT" ] || { echo "no $PT: set LCD2_PTABLE" >&2; exit 2; }
[ -x "$HERE/../../target/release/esp32sim" ] || (cd "$HERE/../.." && cargo build --release -p esp32sim)
ELF=""
[ -n "$LCD2_ELF" ] && ELF="--elf $LCD2_ELF"
exec "$HERE/../../target/release/esp32sim" --chip s3 --boot rom --board waveshare-s3-touch-lcd-2 \
    --flash-mb 16 --psram-mb 8 --bootloader "$BL" --ptable "$PT" --app "$B/$APP" $ELF \
    --console usb --no-dump "$@"
