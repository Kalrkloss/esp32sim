//! Waveshare ESP32-S3-Touch-LCD-2: ST7789T3 240×320 IPS panel on GP-SPI2 (SCLK 39, MOSI 38,
//! MISO 40, CS 45, D/C 42, no reset line, backlight 1), CST816D touch at I2C0 0x15 and a
//! QMI8658 IMU at 0x6B on SDA 48 / SCL 47, battery divider on GPIO 5. The camera header, the
//! TF card and the ADC are not modelled.
//!
//! The panel is a plain 4-wire SPI module behind the controller's own chip select: the D/C pin
//! is the only framing the controller sees, so the board follows it from the GPIO writes and
//! hands every byte a GP-SPI2 transaction produces to the DCS parser in order.
use esp_soc::board::BoardModel;
use esp_soc::devices::DcsPanel;

pub const PIN_LCD_SCLK: u8 = 39;
pub const PIN_LCD_MOSI: u8 = 38;
pub const PIN_LCD_MISO: u8 = 40;
pub const PIN_LCD_CS: u8 = 45;
pub const PIN_LCD_DC: u8 = 42;
pub const PIN_LCD_BL: u8 = 1;
pub const PIN_I2C_SDA: u8 = 48;
pub const PIN_I2C_SCL: u8 = 47;
pub const PIN_BATTERY_ADC: u8 = 5;
/// CST816D and QMI8658 share the touch/IMU I2C bus.
pub const TOUCH_ADDRESS: u8 = 0x15;
pub const IMU_ADDRESS: u8 = 0x6b;

/// What the glass shows: the controller's whole 240×320 RAM in its own orientation (MADCTL's
/// MV/MX/MY already applied when the pixel was written), nothing while it is asleep or the
/// display is off. INVON is not applied: the module is IPS and the driver sets it to compensate
/// the panel's polarity, so the RGB565 the firmware sends is the colour the glass shows.
pub fn lcd_visible(p: &DcsPanel) -> Vec<u16> {
    if !p.on || p.sleeping {
        return vec![0; p.cols * p.rows];
    }
    p.gram.clone()
}

pub struct WaveshareS3TouchLcd2 {
    /// the ST7789T3 behind the 2" glass; `lcd_visible` is what it shows
    pub panel: DcsPanel,
    /// backlight enable (GPIO 1), tracked for the report
    pub backlight: bool,
    pub gpio_events: u64,
    /// pointer state the CST816D reports
    pub touch_state: std::sync::Arc<std::sync::Mutex<crate::i2c::TouchState>>,
    /// board clock and motion selection seen by the QMI8658
    pub imu: std::sync::Arc<crate::i2c::ImuMotion>,
}

impl Default for WaveshareS3TouchLcd2 { fn default() -> Self { Self::new() } }

impl WaveshareS3TouchLcd2 {
    pub fn new() -> Self {
        Self {
            panel: DcsPanel::st7789(),
            backlight: false,
            gpio_events: 0,
            touch_state: Default::default(),
            imu: Default::default(),
        }
    }
}

impl BoardModel for WaveshareS3TouchLcd2 {
    fn name(&self) -> &'static str { "waveshare-s3-touch-lcd-2" }
    fn gpio_changes(&mut self, changes: &[(u8, bool)]) {
        for &(pin, level) in changes {
            self.gpio_events += 1;
            match pin {
                PIN_LCD_DC => self.panel.dc = level,
                PIN_LCD_BL => self.backlight = level,
                _ => {}
            }
        }
    }
    fn gpio_events(&self) -> u64 { self.gpio_events }
    /// The panel is the only device on the SPI bus and the driver picks the host, so every
    /// transfer on either GP-SPI host reaches it; MOSI is all it needs (the module's MISO is
    /// unconnected, which reads as 0xFF).
    fn spi_transfer(&mut self, host: u8, tx: &[u8], rx_len: usize) -> Vec<u8> {
        if host == 2 || host == 3 { for &b in tx { self.panel.byte(b); } }
        vec![0xff; rx_len]
    }
    fn i2c_devices(&mut self) -> Vec<(u8, u8, Box<dyn crate::i2c::I2cDevice>)> {
        use crate::i2c::*;
        vec![
            (0, TOUCH_ADDRESS, Box::new(Cst816d::new(self.touch_state.clone()))),
            (0, IMU_ADDRESS, Box::new(Qmi8658::new(self.imu.clone()))),
        ]
    }
    fn display(&self) -> Option<(u32, u32, Vec<u16>, u64)> {
        Some((self.panel.cols as u32, self.panel.rows as u32, lcd_visible(&self.panel), self.panel.pixels_written))
    }
    fn display_version(&self) -> u64 { self.panel.pixels_written }
    fn display_frames(&self) -> u64 { self.panel.frames }
    fn gram(&self) -> Option<(Vec<u16>, usize, usize)> { Some((self.panel.gram.clone(), self.panel.cols, self.panel.rows)) }
    fn touch(&mut self, x: u16, y: u16, down: bool) {
        self.touch_state.lock().expect("LCD-2 touch state mutex poisoned")
            .update(x.min(self.panel.cols as u16 - 1), y.min(self.panel.rows as u16 - 1), down);
    }
    fn set_imu_motion(&mut self, mode: u32) -> bool { self.imu.mode.store(mode, std::sync::atomic::Ordering::Relaxed); true }
    fn imu_motion(&self) -> u32 { self.imu.mode.load(std::sync::atomic::Ordering::Relaxed) }
    fn report(&self) -> String {
        let p = &self.panel;
        format!("[emu] lcd2: {} RAMWR, {} pixels, madctl={:#x} colmod={:#x} inverted={} on={} sleeping={} backlight={} bbox={:?} top colours {:x?}; gpio events {}",
                p.frames, p.pixels_written, p.madctl, p.colmod, p.inverted, p.on, p.sleeping, self.backlight, p.bbox(), p.histogram(5), self.gpio_events)
    }
}

#[cfg(test)]
mod lcd2_tests {
    use super::*;
    use esp_soc::board::BoardModel;

    fn dc(board: &mut WaveshareS3TouchLcd2, level: bool) { board.gpio_changes(&[(PIN_LCD_DC, level)]); }
    fn command(board: &mut WaveshareS3TouchLcd2, c: u8) { dc(board, false); board.spi_transfer(2, &[c], 1); }
    fn data(board: &mut WaveshareS3TouchLcd2, bytes: &[u8]) { dc(board, true); board.spi_transfer(2, bytes, 0); }

    #[test]
    fn the_panel_is_a_full_size_st7789_and_the_board_answers_to_its_names() {
        let board = WaveshareS3TouchLcd2::new();
        assert_eq!((board.panel.cols, board.panel.rows), (240, 320));
        assert_eq!(board.panel.resets, 0);
        for name in ["waveshare-s3-touch-lcd-2", "waveshare-lcd2", "lcd2"] {
            assert_eq!(crate::board::make_board(name).unwrap().name(), "waveshare-s3-touch-lcd-2");
        }
        assert!(crate::board::make_board("waveshare-s3-touch-lcd-1").is_none());
    }

    /// A driver's own sequence — sleep out, display on, window, RAMWR — reaches the glass through
    /// GP-SPI2 with D/C on its GPIO, and the frame is the full 240×320, uncut.
    #[test]
    fn spi2_transactions_place_pixels_anywhere_in_the_full_frame() {
        let mut board = WaveshareS3TouchLcd2::new();
        command(&mut board, 0x11);                                  // SLPOUT
        command(&mut board, 0x29);                                  // DISPON
        command(&mut board, 0x2a); data(&mut board, &[0, 0, 0, 239]);    // CASET 0..239
        command(&mut board, 0x2b); data(&mut board, &[0, 0, 1, 0x3f]);   // RASET 0..319
        command(&mut board, 0x2c);                                   // RAMWR
        data(&mut board, &[0xf8, 0x00, 0x07, 0xe0, 0x00, 0x1f, 0xff, 0xff]);   // red, green, blue, white

        let (w, h, frame, written) = board.display().expect("the board has a panel");
        assert_eq!((w, h, written), (240, 320, 4));
        assert_eq!(&frame[..4], [0xf800, 0x07e0, 0x001f, 0xffff]);
        assert_eq!(board.panel.bbox(), Some((0, 0, 3, 0)));

        // the last row and column of the frame exist, so a full-screen frame is not cropped
        command(&mut board, 0x2a); data(&mut board, &[0, 239, 0, 239]);
        command(&mut board, 0x2b); data(&mut board, &[1, 0x3f, 1, 0x3f]);
        command(&mut board, 0x2c); data(&mut board, &[0x12, 0x34]);
        assert_eq!(board.panel.gram[319 * 240 + 239], 0x1234, "row 319, column 239 is in the glass");
        assert_eq!(board.panel.pixels_written, 5);
    }

    /// The glass is dark while the controller sleeps or the display is off — as on the module —
    /// and INVON does not change the colour the firmware asked for.
    #[test]
    fn a_sleeping_or_off_panel_shows_nothing_and_inversion_keeps_the_colours() {
        let mut board = WaveshareS3TouchLcd2::new();
        command(&mut board, 0x2c); data(&mut board, &[0x00, 0x1f]);
        assert_eq!(board.display().unwrap().2[0], 0, "power-on: asleep");

        command(&mut board, 0x29);                                  // DISPON while asleep
        assert_eq!(board.display().unwrap().2[0], 0, "still asleep");

        command(&mut board, 0x11);                                  // SLPOUT
        command(&mut board, 0x21);                                  // INVON, as an IPS module's driver sets it
        assert_eq!(board.display().unwrap().2[0], 0x001f, "IPS inversion compensates the panel, it does not repaint the frame");

        command(&mut board, 0x28);                                  // DISPOFF
        assert_eq!(board.display().unwrap().2[0], 0, "display off");
    }

    #[test]
    fn only_the_dc_line_frames_a_transaction_and_the_backlight_is_tracked() {
        let mut board = WaveshareS3TouchLcd2::new();
        board.gpio_changes(&[(PIN_LCD_BL, true), (PIN_LCD_CS, false), (PIN_LCD_SCLK, true)]);
        assert!(board.backlight);
        assert_eq!(board.gpio_events(), 3);
        assert_eq!(board.spi_transfer(2, &[0x29, 0x00], 1), [0xff], "MISO is unconnected");
    }

    #[test]
    fn the_touch_controller_and_imu_are_on_the_touch_i2c_bus() {
        let mut board = WaveshareS3TouchLcd2::new();
        let addresses: Vec<_> = board.i2c_devices().into_iter().map(|(bus, address, _)| (bus, address)).collect();
        assert_eq!(addresses, [(0, TOUCH_ADDRESS), (0, IMU_ADDRESS)]);
    }

    /// A pointer event reaches the CST816D's report, clamped to the glass.
    #[test]
    fn a_touch_becomes_a_cst816d_report() {
        use crate::i2c::I2cDevice;
        let mut board = WaveshareS3TouchLcd2::new();
        board.touch(300, 400, true);
        let mut device = board.i2c_devices().into_iter().next().expect("touch controller").2;

        fn read_at(device: &mut Box<dyn I2cDevice>, reg: u8) -> u8 {
            assert!(device.start(false));
            assert!(device.write(reg));
            assert!(device.start(true));
            device.read()
        }
        assert_eq!(read_at(&mut device, 0x02), 1, "one finger down");
        assert_eq!(read_at(&mut device, 0x03), 0, "x high nibble");
        assert_eq!(read_at(&mut device, 0x04), 239, "x clamped to the glass");
        assert_eq!(read_at(&mut device, 0x05), 1, "y high nibble");
        assert_eq!(read_at(&mut device, 0x06), 0x3f, "y clamped to the glass, low byte");
    }
}
