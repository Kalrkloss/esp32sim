# Waveshare ESP32-S3-Touch-LCD-2

The 2" touch board: an ST7789T3 240×320 IPS panel on GP-SPI2, CST816D capacitive touch and a
QMI8658 IMU on one I2C bus, 16 MB flash and 8 MB of octal PSRAM. The board model is in
`esp32s3/src/board/waveshare_lcd2.rs`; `docs/boards.md` lists what each device models.

    LCD2_BUILD=~/work/lcd2/build LCD2_APP=lcd2.bin examples/waveshare-s3-touch-lcd-2/run.sh \
        --web 8770 --tft-png lcd2.png

`--tft-png` writes what the glass shows (240×320, RGB565 as the firmware sent it). A script can
drive the panel's touch controller:

    printf '6 touch 120 160 1\n6.2 touch 120 160 0\n6.5 stop\n' > /tmp/lcd2-touch.txt
    LCD2_BUILD=~/work/lcd2/build LCD2_APP=lcd2.bin examples/waveshare-s3-touch-lcd-2/run.sh \
        --script /tmp/lcd2-touch.txt --tft-png after-tap.png

Coordinates are the panel's own (240 wide, 320 high) — the driver applies whatever rotation it
programmed. `--flash-file img.bin` keeps the flash between runs, which is what firmware that stores
its configuration in NVS needs (see `docs/evidence/persistent-flash`).

The images this board was brought up on are released ESP32-S3 builds with the vendor's own
`bootloader_opi_80m.bin` and a 16 MB table; both are properties of the image, not of the board, so
point the script at whatever build you have.
