#!/bin/sh
# Both radio roles in one run: join the access point the guest brings up, and offer one it can see
# and join. The paths come from the environment so no image is baked in.
#
#   APP=app.bin BOOTLOADER=bootloader.bin PTABLE=ptable.bin BOOT_APP0=boot_app0.bin \
#   EMU=target/release/esp32sim SSID=bench PSK=bench-password ./run-two-roles.sh --max-seconds 75
#
# Two forwards: the first reaches the guest while it is the access point, the second once it has
# rebooted into a station of the emulated one. Extra flags are appended.
set -e
EMU=${EMU:-target/release/esp32sim}
: "${APP:?set APP to the guest's application image}"
: "${BOOTLOADER:?set BOOTLOADER to its bootloader image}"
: "${PTABLE:?set PTABLE to its partition table}"
: "${SSID:?set SSID to the access point the guest should join}"
: "${PSK:?set PSK to its passphrase}"

exec "$EMU" --chip "${CHIP:-s3}" --boot rom \
  --bootloader "$BOOTLOADER" --ptable "$PTABLE" --app "$APP" \
  ${BOOT_APP0:+--flash-at "0xe000=$BOOT_APP0"} \
  --flash-mb "${FLASH_MB:-16}" --psram-mb "${PSRAM_MB:-8}" \
  --board "${BOARD:-none}" --console "${CONSOLE:-uart0}" \
  --wifi "ssid=$SSID,chan=${CHANNEL:-6},psk=$PSK" \
  --wifi-join "${JOIN:-}" \
  --hostfwd "tcp:127.0.0.1:${AP_PORT:-18080}-192.168.4.1:80" \
  --hostfwd "tcp:127.0.0.1:${STA_PORT:-18081}-10.0.2.15:80" \
  "$@"
