# Guition JC3248W535 · LVGL on an emulated AXS15231B QSPI panel

    ./run-lvgl.sh --tft-png page1.png --max-seconds 10
    ./run-lvgl.sh --script swipe.txt --tft-png page3.png --max-seconds 12
    ./run-lvgl.sh --script tap.txt   --tft-png taps.png  --max-seconds 12

The board model (`--board jc3248w535`) is a 320×480 AXS15231B on QSPI with the same IC's
capacitive touch on I2C0 at 0x3B. The application is MicroPython + LVGL: three tileview pages in
different colours, so one screenshot says whether the panel is being driven, and a button on page 2
that counts taps. `swipe.txt` swipes left twice, `tap.txt` swipes to page 2 and taps the button
twice.

| File | What it is |
| --- | --- |
| `fs/main.py` | the application: display + touch init, the tileview, the tap counter, the LVGL loop |
| `swipe.txt`, `tap.txt` | `--script` files (emulated seconds, `touch X Y 1|0`) |
| `tools/make_fs.py` | builds the FAT12 image the firmware mounts as its VFS (4096-byte sectors) |
| `run-lvgl.sh` | builds that image and runs the emulator with it |

## The image

The panel needs a MicroPython build with `lv_binding_micropython` and the QSPI bus driver that
chunks a full frame (`SPIBusFast`, 307200 bytes exceeds the SPI transfer limit on hardware).
Built and verified with upstream `lvgl_micropython` at `d2d2646`:

    cd /opt/lvgl_micropython
    source /opt/esp-idf/export.sh
    env -u PYTHONPATH python3 make.py esp32 BOARD=ESP32_GENERIC_S3 BOARD_VARIANT=SPIRAM_OCT \
        --flash-size=16 DISPLAY=axs15231b INDEV=axs15231 --ota --partition-size=4194304

That produces `build/lvgl_micropy_ESP32_GENERIC_S3-SPIRAM_OCT-16.bin` (a whole flash image) with
`ota_0` at 0x20000, a 4 MB app partition and a 0x7E0000 `vfs` FAT partition at 0x820000.
`run-lvgl.sh` reads the image path from `$LVGL_MICROPYTHON_IMAGE`.

Two fixes are needed in the build tree's frozen drivers — both are upstream defects, not
emulator-specific:

* `api_drivers/common_api_drivers/display/axs15231b/axs15231b.py` — `SPIBusFast` is a separate C
  type, so `isinstance(bus, lcd_bus.SPIBus)` rejects it ("incompatable bus driver") and the QSPI
  detection would send 8-bit commands. Accept `getattr(lcd_bus, "SPIBusFast", None)` as well, and
  `import time` (its `reset()` calls `time.sleep_ms`).
* `api_drivers/common_api_drivers/indev/axs15231.py` — the default `startup_rotation` uses `lv`
  without importing `lvgl`, so `import axs15231` raises `NameError`.

The application and the drivers are loaded from the VFS, not frozen, so a screen change costs a
filesystem rebuild, not a firmware build. `tools/make_fs.py` writes FAT12 with 4096-byte sectors
because that is what MicroPython's `Partition` block device reports (`MICROPY_FATFS_MAX_SS` is
4096, immovable flash erases in 4096-byte sectors); it also writes the long-file-name entries for
`axs15231b.py`.

## What the panel model does

* The QSPI framing: opcode `0x02` (write command) or `0x32` (write memory), the DCS command in the
  24-bit address word — `02 00 2A 00 <4 bytes>` is CASET, `32 00 2C 00 <pixels>` is RAMWR. A DCS
  RAMWRC (`0x3C`) continues the address counter instead of restarting it at the window origin.
* Long writes split over several DMA transactions: the first carries the header, the rest are pure
  payload. The model takes the controller's command/address phase split rather than guessing, since
  a pixel byte can be `0x02` or `0x32` too.
* Full frames only, as the board needs: every flush restarts at (0, 0) and fills the whole 320×480
  GRAM. `QspiPanel::ignoring_window()` additionally drops CASET/RASET for the batches that ignore
  window commands.
* Touch: the 11-byte poll command (`B5 AB A5 5A 00 00 00 08 00 00 00`) followed by the eight-byte
  report. A press reports one point with the event bits `0x80`, the lift is one report with `0xff`
  and event `0x40`, and an untouched controller returns the idle pattern `85 85 …`.

Not modelled: the QSPI bus timing, the panel's own oscillator/refresh, tearing, brightness
(`0x51` is accepted and ignored), and the touch gesture codes (only single-point coordinates).

Not wired up yet: the browser UI's layout switch (`web/run.html`) knows the Waveshare boards but
not this one, so `--web` shows the console without the panel; `--tft-png` and the MCP
`sim_screenshot` path do produce the frame.
