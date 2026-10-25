# main.py -- LVGL screen for the JC3248W535 panel model in esp32sim.
#
# Needs a MicroPython image built with DISPLAY=axs15231b and INDEV=axs15231: the 320x480 panel
# hangs on QSPI (SCK 47, D0..D3 21/48/40/39, CS 45, backlight 1) and the touch controller of the
# same IC on I2C0 at 0x3B (SDA 4, SCL 8). Two full PSRAM framebuffers, because this panel only
# refreshes whole frames through the chunking bus driver (SPIBusFast).
#
# Three tileview pages in different colours make a swipe obvious in a screenshot; the button on
# page 2 counts taps, so a tap and a swipe are told apart by what the screen shows afterwards.

import time

import i2c
import lcd_bus
import machine
import lvgl as lv

from axs15231 import AXS15231
from axs15231b import AXS15231B

WIDTH = 320
HEIGHT = 480
TOUCH_ADDRESS = 0x3B

PAGES = (
    (0x12406F, "Page 1", "swipe left"),
    (0x0F6B3C, "Page 2", "tap the button"),
    (0x6B3F0F, "Page 3", "swipe right"),
)


def start_display():
    spi_bus = machine.SPI.Bus(host=1, sck=47, quad_pins=(21, 48, 40, 39))
    bus = lcd_bus.SPIBusFast(spi_bus=spi_bus, dc=-1, cs=45, freq=40_000_000, spi_mode=3, quad=True)
    size = WIDTH * HEIGHT * 2
    frame1 = bus.allocate_framebuffer(size, lcd_bus.MEMORY_SPIRAM)
    frame2 = bus.allocate_framebuffer(size, lcd_bus.MEMORY_SPIRAM)
    display = AXS15231B(bus, WIDTH, HEIGHT, frame_buffer1=frame1, frame_buffer2=frame2,
                        backlight_pin=1, color_space=lv.COLOR_FORMAT.RGB565, rgb565_byte_swap=True)
    display.set_power(True)
    display.set_backlight(80)
    display.init()
    print("jc3248w535: display init ok (%dx%d, two full framebuffers)" % (WIDTH, HEIGHT))
    return bus, display


def start_touch():
    # The touch driver needs a running display; no reset or interrupt line on this board.
    i2c_bus = i2c.I2C.Bus(host=0, sda=4, scl=8, freq=400_000)
    device = i2c.I2C.Device(i2c_bus, TOUCH_ADDRESS, 8)
    touch = AXS15231(device)
    print("jc3248w535: touch init ok (address 0x%02X)" % TOUCH_ADDRESS)
    return touch


def build_ui():
    screen = lv.screen_active()
    screen.set_style_bg_color(lv.color_hex(0x000000), 0)
    screen.set_style_bg_opa(lv.OPA.COVER, 0)
    view = lv.tileview(screen)
    view.set_size(WIDTH, HEIGHT)
    view.set_style_bg_opa(lv.OPA.COVER, 0)
    tiles = []
    for index, (colour, title, hint) in enumerate(PAGES):
        tile = view.add_tile(index, 0, lv.DIR.RIGHT)
        # A tile is a plain lv_obj: a background colour alone stays transparent under the theme.
        tile.set_style_bg_color(lv.color_hex(colour), 0)
        tile.set_style_bg_opa(lv.OPA.COVER, 0)
        heading = lv.label(tile)
        heading.set_text(title)
        heading.set_style_text_font(lv.font_montserrat_28, 0)
        heading.set_style_text_color(lv.color_hex(0xFFFFFF), 0)
        heading.align(lv.ALIGN.CENTER, 0, -60)
        note = lv.label(tile)
        note.set_text(hint)
        note.set_style_text_color(lv.color_hex(0xDDE3EA), 0)
        note.align(lv.ALIGN.CENTER, 0, -10)
        tiles.append(tile)
    view.set_tile_by_index(0, 0, 0)
    return view, tiles


def add_tap_counter(tile):
    button = lv.button(tile)
    button.set_size(180, 64)
    button.align(lv.ALIGN.CENTER, 0, 70)
    label = lv.label(button)
    label.set_text("taps: 0")
    label.center()
    counter = [0]

    def clicked(event):
        counter[0] += 1
        label.set_text("taps: %d" % counter[0])
        print("jc3248w535: tap %d" % counter[0])

    button.add_event_cb(clicked, lv.EVENT.CLICKED, None)
    return counter


bus, display = start_display()
touch = start_touch()
view, tiles = build_ui()
add_tap_counter(tiles[1])
print("jc3248w535: ui ready")

last_tick = time.ticks_ms()
while True:
    now = time.ticks_ms()
    lv.tick_inc(time.ticks_diff(now, last_tick))
    last_tick = now
    lv.timer_handler()
    time.sleep_ms(5)
