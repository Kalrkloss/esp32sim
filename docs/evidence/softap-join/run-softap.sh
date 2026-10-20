#!/bin/sh
# Run a soft-AP guest image in the emulator, join the access point it brings up, and carry a host
# listener to its web server. The paths come from the environment so no bench is baked in.
#
#   APP=app.bin BOOTLOADER=bootloader.bin PTABLE=ptable.bin BOOT_APP0=boot_app0.bin \
#   EMU=target/release/esp32sim ./run-softap.sh
#
# Extra flags (for example `--max-seconds 45 --tft-png out.png`) are appended to the emulator's.
set -e
EMU=${EMU:-target/release/esp32sim}
: "${APP:?set APP to the guest's application image}"
: "${BOOTLOADER:?set BOOTLOADER to its bootloader image}"
: "${PTABLE:?set PTABLE to its partition table}"
PORT=${PORT:-18080}

exec "$EMU" --chip "${CHIP:-s3}" --boot rom \
  --bootloader "$BOOTLOADER" --ptable "$PTABLE" --app "$APP" \
  ${BOOT_APP0:+--flash-at "0xe000=$BOOT_APP0"} \
  --flash-mb "${FLASH_MB:-16}" --psram-mb "${PSRAM_MB:-8}" \
  --board "${BOARD:-none}" --console uart0 \
  --wifi-join "${JOIN:-}" \
  --hostfwd "tcp:127.0.0.1:$PORT-192.168.4.1:80" \
  "$@"
