# A 240×320 panel and a CST816D that a firmware talks to

Two claims, one board model:

1. An unmodified released ESP32-S3 image, configured through its own web UI, draws its boot splash
   **complete on a 240×320 glass** — every column and row, not a 240×240 or 135×240 window.
2. Pointer events the emulator injects as script actions **reach the firmware through the touch
   controller**: the firmware's own BASIC loop prints the coordinates that were injected.

Both are properties of the board model (`esp32s3/src/board/waveshare_lcd2.rs` plus `Cst816d` in
`esp32s3/src/i2c.rs`), not of the firmware: the image only had to be told which pins it is on.

## Inputs

| Input | Identity |
| --- | --- |
| Application image | `Annex32-S3_2.99.27_qio_qspi.bin`, 2 317 696 B, SHA-256 `128d0209a236ecd3ca30a51b4eb9e3e00503bbf3a7a1c71bb472c017c88d6bc6` |
| Bootloader | `bootloader_opi_80m.bin`, SHA-256 `7c2892c8bea5762e3c9f44cfbae154d01f2da93dea97f9238fea4b02a08d35ed` |
| Partition table | `ptable-16mb.bin`, SHA-256 `24db2c95993e99f85fb925b8d8d722b113e907b1a7d8597656d233415944c5c6` |
| `otadata` seed | `boot_app0.bin`, SHA-256 `f94c5d786a7a8fab06ac5d10e33bf37711a6697636dc037559ea19cc410a17f0` |
| Emulator | branch `lcd2-board`, the board commit on top of the `splash2` bench build (join + `--hostfwd` + panel over GP-SPI2 + `--flash-file`) |
| Host | x86-64, Linux 6.12, 4 CPUs, no silicon attached |

The image is ESP32-S3 code for 16 MB flash and 8 MB octal PSRAM; the bootloader is the octal-PSRAM one
the image needs. `--flash-mb 16 --psram-mb 8` are therefore part of every command below.

## Phase A — configure through the firmware's own web UI

The emulator joins the access point the firmware brings up and carries a host listener into the
guest's subnet:

```sh
esp32sim --chip s3 --boot rom --rom roms/esp32s3_rev0_rom.elf \
  --bootloader bootloader_opi_80m.bin --ptable ptable-16mb.bin \
  --app Annex32-S3_2.99.27_qio_qspi.bin --flash-at 0xe000=boot_app0.bin \
  --flash-mb 16 --psram-mb 8 --board waveshare-s3-touch-lcd-2 --console usb --no-dump \
  --wifi-join '' --hostfwd tcp:127.0.0.1:18120-192.168.4.1:80 \
  --flash-file /tmp/annex-lcd2.img --max-seconds 120
```

`POST /_kfg_` with an empty body reads the whole settings record; writing it back with these values is
the configuration (a subset clears the rest, so the record is round-tripped). On a fresh flash image the
first read is the factory record — `VER=Annex32-S3 2.99.27 qio qspi&…&MODULE=0&DISPLAY=255&ORIENTATION=1&SDCARD=0&TOUCH=0&I2CPORT=1&PINS=-1,…` —
i.e. no TFT, no touch, and the run below is what changes that:

```
MODULE=1            CUSTOM, the only mode where pin settings apply
DISPLAY=105         ST7789 #5 — the 240x320 preset (see the table below)
TOUCH=5             CST816 (I2C)
I2CPORT=0           the touch bus is shared (the firmware then reports "I2C PORT for TOUCH CTS816: 0 48 47")
ORIENTATION=0
PINS=38,40,39,45,42,-1,1,-1,-1,-1,-1,-1,48,47,-1,-1,-1,-1,-1,-1,
                    MOSI 38, MISO 40, SCLK 39, CS 45, DC 42, RST -1, BL 1, TOUCH -1, SD -1,
                    I2S -1 -1 -1, SDA 48, SCL 47
```

`GET /config?restart=yes` restarts the firmware; the settings live in NVS in the flash image, so the
next run boots configured.

## The preset that fills the glass

The firmware prints the geometry it selected (`display res W H orientation N`) on every boot, so the
presets can be told apart without a capture. ST7789 presets on this image, `ORIENTATION=0`:

| `DISPLAY` | Preset | Geometry |
| --- | --- | --- |
| 89 | ST7789 #0 | 240×240 |
| 101 | ST7789 #1 | 135×240 |
| 102 | ST7789 #2 | 240×280 |
| 103 | ST7789 #3 | 172×320 |
| 104 | ST7789 #4 | 170×320 |
| **105** | **ST7789 #5** | **240×320** |
| 106 | ST7789 #6 | 240×320 |
| 107 | ST7789 #7 | 240×240 |
| 108 | ST7789 #8 | 170×320 |

`ST7735 #0` (`DISPLAY=0`, the control) is 160×80 and is what the earlier receipt used on the Atech
board: it draws the same splash inside a 160×80 window. The failure mode this receipt rules out is not
"no splash" but "a splash that is cut off": with `DISPLAY=89` the splash occupies rows 0..239 and the
bottom 80 rows of the glass stay black, because the firmware believes the panel is 240×240.

## Phase B — the frame

```sh
esp32sim ... --board waveshare-s3-touch-lcd-2 --flash-file /tmp/annex-lcd2.img \
  --max-seconds 22 --tft-png /tmp/annex-lcd2-105.png
```

End-of-run report from the board:

```
[emu] lcd2: 3989 RAMWR, 198007 pixels, madctl=0x8 colmod=0x55 inverted=false on=true sleeping=false
      backlight=false bbox=Some((0, 0, 239, 319)) top colours [(c1d, a55d), (ffff, 2661), (c1e, 771),
      (3fd, 51c), (c3e, 4f1)]; cst816d 4119 register reads; gpio events 20859
```

`bbox` covers the whole 240×320 RAM and the most common colour is `0x0c1d` (the splash's blue), so the
frame is neither a window inside the glass nor a single flat fill. `--tft-png` writes
`annex-lcd2-105.png`: 240×320, 230 803 B, SHA-256
`c23feebb96195c6f42b50ae568256882bf0d452bc6b21273fe4a30d53195c87b` — two independent runs of the
capture produced the same hash, so the frame is deterministic. It shows the blue splash with the bee,
the copyright line, `ACCESS POINT`, `IP : 192.168.4.1` and the QR code, all complete: no edge is cut.
The image file is not committed; `annex-lcd2.py` in this directory reproduces it.

`madctl=0x08` is BGR only — no MV/MX/MY — so the driver is driving the glass in its native portrait
orientation, and `inverted=false` means this preset does not send INVON. The model keys the displayed
frame on `on`/`sleeping` and leaves INVON out of the colour path on purpose: the module is IPS and a
driver that sets INVON there is compensating the panel's polarity, so the RGB565 the firmware sends is
what the glass shows. The unit test `a_sleeping_or_off_panel_shows_nothing_and_inversion_keeps_the_colours`
pins both halves of that.

## The touch trace

The firmware's own BASIC interpreter polls the controller. The program is saved into the guest through
its editor's WebSocket save handshake (`save:start` / `SAVE:GIVE` / `save:more` / `SAVE:END`), loaded
and started with `cmd:run`; the emulator's script injects a tap and a swipe in emulated time:

```
12.0  touch 120 160 1      # tap, down
12.25 touch 120 160 0      # tap, up
16.0  touch 60 200 1       # swipe, down at the left
16.15 touch 230 200 1      # swipe, moved right (still down)
16.30 touch 230 200 0      # swipe, up
```

BASIC program (`/touch.bas` on the guest):

```basic
print "TOUCHMON up"
do
  touch.raw
  if touch.z > 0 then
    print "RAW z=";touch.z;" x=";touch.x;" y=";touch.y
  endif
  touch.read
  if touch.z > 0 then
    print "CAL z=";touch.z;" x=";touch.x;" y=";touch.y
  endif
  pause 30
loop
```

The firmware's console, from its own UART:

```
Program Running
TOUCHMON up
RAW z=1 x=120 y=160        (x8, the tap)
RAW z=1 x=60 y=200         (x5, the swipe's press)
RAW z=1 x=230 y=200        (x4, the swipe's end)
CAL z=1 x=0 y=0            (x8, the tap through TOUCH.READ)
```

Readings:

- The controller answers the finger count at 0x02 and the point at 0x03..0x06 with the coordinates the
  emulator injected, in the panel's own 240×320 space, so the driver's I2C transactions reach the
  model and its register layout is the vendor BSP's.
- `TOUCH.READ` returns 0,0 because it returns *calibrated* coordinates and no calibration has been
  stored in this image; `TOUCH.RAW` is the controller's own answer. That is a property of the image's
  calibration state, not of the transport, and it is why the receipt quotes `TOUCH.RAW`.
- The board's end-of-run report counts the driver's register reads (4119 in the run above), which is
  the quantitative form of the same claim: the firmware polls this controller, it does not read a
  stub answered by the bus model.
- The swipe's intermediate coordinate (230,200) is read while the press is still down, i.e. the model
  reports a held point and its movement, not only press/release edges.

## Tests

`cargo test -p esp32s3` (release):

- `spi2_transactions_place_pixels_anywhere_in_the_full_frame` — a driver sequence over GP-SPI2 with
  D/C on GPIO 42 writes the four corners' colours and the last row and column of the 240×320 glass.
- `a_sleeping_or_off_panel_shows_nothing_and_inversion_keeps_the_colours` — the glass is dark while
  the controller sleeps or the display is off, and INVON does not repaint the frame.
- `only_the_dc_line_frames_a_transaction_and_the_backlight_is_tracked`, `the_panel_is_a_full_size_st7789_and_the_board_answers_to_its_names`,
  `the_touch_controller_and_imu_are_on_the_touch_i2c_bus`, `a_touch_becomes_a_cst816d_report`.
- `cst816d_answers_the_vendor_probe_and_reports_one_point`, `cst816d_turns_the_pointer_stream_into_gestures`,
  `cst816d_keeps_a_tap_the_driver_never_sampled` — the register set, the gesture register, and a tap
  shorter than the driver's poll interval.

The two scripts that produce the two halves of this receipt are next to this file: `annex-lcd2.py`
(configure through the web UI, then capture the frame) and `annex-touch.py` (save the BASIC program,
run it, inject the tap and the swipe, print the firmware's console lines). Both take the emulator from
`/root/esp32sim-lcd2/target/release/esp32sim` and the image inputs from `/root/annex-basic`; change
`EMU`/`HERE` at the top for another machine.

The workspace suite and both Clippy runs are recorded in the pull request; no golden output changed.

## What this does not cover

- No silicon: the panel's colour, inversion and offset behaviour is modelled from the module's
  controller and driver, not measured on the glass.
- The gesture register is modelled but the firmware used here never reads it; no evidence is claimed
  for gestures above the register level.
- The TF card, the camera header and the battery divider on GPIO 5 are not modelled.
- Backlight: this image never drives GPIO 1 (`backlight=false` in the report), so the model shows the
  frame regardless of that line. A firmware that expects the glass to be dark until the backlight is
  enabled is not covered.
- I2C pin routing is not modelled: the controller answers on bus 0 whatever pins the driver routes,
  which is why the firmware's own `I2C PORT for TOUCH CTS816: 0 48 47` line matters as the check that
  the configured bus is the one the model answers on.

## Experiment entry

[EX235](../experiments.md#ex235), on top of EX234 (the GP-SPI2 feed a board's panel takes).
