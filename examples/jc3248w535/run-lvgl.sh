#!/bin/sh
# Run a MicroPython + LVGL image on the emulated Guition JC3248W535 (AXS15231B over QSPI,
# capacitive touch on I2C0 at 0x3B).
#
#   ./run-lvgl.sh --tft-png page1.png --max-seconds 8
#   ./run-lvgl.sh --script swipe.txt --tft-png page2.png --max-seconds 12
#
# The image itself is a plain MicroPython build with DISPLAY=axs15231b INDEV=axs15231; the
# application and the two driver shims come from a FAT image written into the VFS partition,
# so nothing has to be rebuilt to change the screen. See README.md for the build command.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
EMU=${ESP32SIM:-$HERE/../../target/release/esp32sim}
IMAGE=${LVGL_MICROPYTHON_IMAGE:-/opt/lvgl_micropython/build/lvgl_micropy_ESP32_GENERIC_S3-SPIRAM_OCT-16.bin}
# The partition table of that image: `vfs,data,fat,0x820000,0x7E0000`.
VFS_OFFSET=${VFS_OFFSET:-0x820000}
VFS_SIZE_BYTES=${VFS_SIZE_BYTES:-8257536}
WORK=${ESP32SIM_JC3248W535_WORK:-${XDG_CACHE_HOME:-$HOME/.cache}/esp32sim-jc3248w535}

if [ ! -f "$IMAGE" ]; then
    echo "run-lvgl: no MicroPython image at $IMAGE (set LVGL_MICROPYTHON_IMAGE)" >&2
    exit 2
fi

mkdir -p "$WORK"
FS="$WORK/vfs.img"
if [ ! -f "$FS" ] || [ -n "$(find "$HERE/fs" -newer "$FS" -type f 2>/dev/null | head -n 1)" ]; then
    python3 "$HERE/tools/make_fs.py" "$HERE/fs" "$FS" --size "$VFS_SIZE_BYTES" >&2
fi

exec "$EMU" --board jc3248w535 --boot rom --flash-mb 16 --psram-mb 8 \
    --flash-image "$IMAGE" --flash-at "$VFS_OFFSET=$FS" \
    --console usb --no-dump "$@"
