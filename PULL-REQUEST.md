# A board for a plain 4-wire SPI panel, with its touch controller

Every S3 board model so far pairs a panel with a bus shape the model already had: the Atech TFT
bit-bangs an ST7735 on GPIO edges, the Touch-LCD-4B feeds an ST7701S from LCD_CAM, the AMOLED speaks
QSPI frames with the command byte embedded in the transfer. The shape that was missing is the ordinary
one: a controller with chip select in hardware, the D/C line as the only framing, and pixels arriving
as plain GP-SPI2 transactions — which is what Arduino_GFX, TFT_eSPI, `esp_lcd` and MicroPython display
drivers produce.

`--board waveshare-s3-touch-lcd-2` adds it (`esp32s3/src/board/waveshare_lcd2.rs`): `DcsPanel::st7789()`
at its native 240×320, D/C on GPIO 42 followed from the GPIO writes, every GP-SPI2 transaction handed to
the DCS parser in order, backlight level on GPIO 1 tracked. The frame the UI and `--tft-png` publish is
the controller's whole RAM, and it is dark while the controller sleeps or the display is off — that is
what the glass does. INVON is tracked but not applied to the published frame: the module is IPS and a
driver that sets inversion there is compensating the panel's polarity, so the RGB565 the firmware sends
is what the glass shows. That is a claim about this module, not a general one, so it is a documented
property of this board and a unit test pins both halves.

Touch is a new `Cst816d` (address 0x15, I2C0): the register set the vendor BSP probes — chip id `0xB6`
at 0xA7, finger count at 0x02, one point at 0x03..0x06 — plus the gesture register at 0x01, derived from
the pointer stream the driver samples (a press that lifts without travel is a single click, travel of
40 px or more is the slide in that direction). A `Qmi8658` stub answers on the same bus so firmware that
probes the IMU does not stall. The board needs `--flash-mb 16 --psram-mb 8`.

Nothing in the model is specific to one firmware: it is a panel and a touch controller on known pins,
so an ESP-IDF, Arduino or MicroPython image can be pointed at it.

## Branch base

Cut from `spi2-panel` (`9072c3d8`), whose panel feed this board uses. The bench that produced the
evidence ran on that branch plus the join, `--hostfwd` and `--flash-file` commits — the image is
configured through its own web UI, which needs all three. None of those are part of this change.

## Evidence

An unmodified released ESP32-S3 image (16 MB flash, 8 MB octal PSRAM, its own bootloader and partition
table), configured through its own web UI:

- **Full-size frame.** With `MODULE=1`, the 240×320 display preset, `PINS` = MOSI 38, MISO 40,
  SCLK 39, CS 45, DC 42, RST -1, BL 1 and the CST816 touch controller, the boot splash lands on the
  whole glass: 3989 RAMWR, 198 007 pixels, bounding box (0,0,239,319), the splash's blue as the most
  common colour, and `--tft-png` a 240×320 image of the blue splash with the bee, the IP and the QR
  code, uncut. Two runs produced the same PNG hash.
- **The preset matters, and the firmware says which one it is.** The image prints
  `display res W H orientation` on every boot, so the nine ST7789 presets were identified: five are not
  240×320, and with a 240×240 preset the same splash occupies the top 240 rows while the bottom 80 rows
  of the glass stay black. That is the difference between a pass and a crop.
- **Touch reaches the firmware.** The firmware's own BASIC loop polls the controller and prints what it
  reads; emulator script actions injected a tap at (120,160) and a swipe from (60,200) to (230,200).
  The firmware printed `RAW z=1 x=120 y=160`, then `x=60 y=200` while the press was held and
  `x=230 y=200` before the lift; the board counted 4119 CST816D register reads. `TOUCH.READ` (the
  calibrated reading) returns 0,0 in this image because no calibration is stored — `TOUCH.RAW` is the
  controller's own answer and is what the receipt quotes.

Receipt with the commands, the settings record, the preset table, the console output and the limits:
[docs/evidence/waveshare-s3-touch-lcd-2](docs/evidence/waveshare-s3-touch-lcd-2/README.md), experiment
row EX235 (on top of EX234).

## Tests

Ten tests, six on the board and four on the device:

- the driver sequence over GP-SPI2 writes the four corner colours and the last row and column of the
  240×320 glass; a sleeping or dark controller publishes nothing; INVON does not repaint the frame;
  D/C is the only framing and MISO reads as 0xFF; the touch controller and IMU are the bus-0
  addresses; a clamped pointer becomes a CST816D report;
- the device answers the vendor probe, reports one point, turns the pointer stream into a click and
  four slides, keeps a tap the driver never sampled, and counts its reads.

## Checks run

- `cargo build --release --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy --release --target wasm32-unknown-unknown -p esp32sim-wasm --features jit-tests -- -D warnings`
- `ESP32SIM_ROM_DIR=roms cargo test --release --workspace -- --include-ignored --skip external_`

No golden output changed: the board is a new module and one arm in `make_board`, so firmware that does
not select it takes the same path and the same instruction count as before.
