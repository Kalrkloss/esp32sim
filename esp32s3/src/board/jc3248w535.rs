use esp_soc::board::{BoardModel, SpiPhases};
use esp_soc::devices::qspi_panel::QspiPanel;

/// QSPI clock (the AXS15231B takes the command and pixel stream on four data lines).
pub const PIN_QSPI_SCK: u8 = 47;
/// QSPI data lines D0..D3.
pub const PIN_QSPI_DATA: [u8; 4] = [21, 48, 40, 39];
pub const PIN_QSPI_CS: u8 = 45;
/// Backlight, dimmed through LEDC.
pub const PIN_BACKLIGHT: u8 = 1;
/// Capacitive touch on the panel IC, on I2C0.
pub const PIN_TOUCH_SDA: u8 = 4;
pub const PIN_TOUCH_SCL: u8 = 8;
/// The AXS15231B's touch half answers here.
pub const TOUCH_ADDRESS: u8 = 0x3b;

/// Guition JC3248W535: ESP32-S3-WROOM-1, 3.5" 320×480 IPS behind an AXS15231B on QSPI, the same
/// IC's capacitive touch on I2C0 (SDA 4 / SCL 8, no reset or interrupt line), 16 MB flash and
/// 8 MB octal PSRAM.
///
/// There is no D/C line: the panel is driven with the QSPI framing (`0x02`/`0x32` opcode plus a
/// 24-bit address word carrying the DCS command), which is why the board takes the phase-aware
/// SPI callback. The panel runs at 0° only; a 180° view is the firmware's business, not MADCTL's.
pub struct Jc3248w535 {
    pub gpio_events: u64,
    pub panel: QspiPanel,
    pub touch_state: std::sync::Arc<std::sync::Mutex<crate::i2c::TouchState>>,
    pub touch_counters: std::sync::Arc<crate::i2c::AxsCounters>,
    pub backlight: bool,
    /// Transactions that reached a SPI host the panel is not on.
    pub ignored_transfers: u64,
}

impl Default for Jc3248w535 { fn default() -> Self { Self::new() } }

impl Jc3248w535 {
    pub const WIDTH: usize = 320;
    pub const HEIGHT: usize = 480;

    pub fn new() -> Self {
        Jc3248w535 {
            gpio_events: 0,
            panel: QspiPanel::axs15231b(),
            touch_state: Default::default(),
            touch_counters: Default::default(),
            backlight: false,
            ignored_transfers: 0,
        }
    }
}

impl BoardModel for Jc3248w535 {
    fn name(&self) -> &'static str { "jc3248w535" }
    fn gpio_changes(&mut self, changes: &[(u8, bool)]) {
        self.gpio_events += changes.len() as u64;
        for &(pin, level) in changes { if pin == PIN_BACKLIGHT { self.backlight = level; } }
    }
    fn gpio_events(&self) -> u64 { self.gpio_events }
    /// The panel decodes framing (opcode plus address word), not a flat byte stream.
    fn uses_spi_phases(&self) -> bool { true }
    fn spi_transfer_phases(&mut self, host: u8, phases: SpiPhases, tx: &[u8], rx_len: usize) -> Vec<u8> {
        if host == 2 { self.panel.transaction(phases, tx); } else { self.ignored_transfers += 1; }
        vec![0xff; rx_len]
    }
    fn i2c_devices(&mut self) -> Vec<(u8, u8, Box<dyn crate::i2c::I2cDevice>)> {
        vec![(0, TOUCH_ADDRESS, Box::new(crate::i2c::Axs15231Touch::with_counters(self.touch_state.clone(), self.touch_counters.clone())))]
    }
    fn display(&self) -> Option<(u32, u32, Vec<u16>, u64)> {
        Some((Self::WIDTH as u32, Self::HEIGHT as u32, self.panel.frame(), self.panel.panel.pixels_written))
    }
    fn display_version(&self) -> u64 { self.panel.panel.pixels_written }
    fn display_frames(&self) -> u64 { self.panel.panel.frames }
    fn gram(&self) -> Option<(Vec<u16>, usize, usize)> {
        Some((self.panel.frame(), Self::WIDTH, Self::HEIGHT))
    }
    fn touch(&mut self, x: u16, y: u16, down: bool) {
        self.touch_state.lock().expect("JC3248W535 touch state mutex poisoned")
            .update(x.min(Self::WIDTH as u16 - 1), y.min(Self::HEIGHT as u16 - 1), down);
    }
    fn report(&self) -> String {
        use std::sync::atomic::Ordering::Relaxed;
        let mut text = self.panel.report();
        text += &format!("\n[emu] touch: {} reports, {} register reads at 0x{TOUCH_ADDRESS:02x}; backlight {}; gpio events {}",
                         self.touch_counters.reports.load(Relaxed), self.touch_counters.reads.load(Relaxed), self.backlight, self.gpio_events);
        if self.ignored_transfers != 0 { text += &format!("\n[emu] {} transactions on SPI hosts without the panel", self.ignored_transfers); }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i2c::Axs15231Touch;

    const FRAMING: SpiPhases = SpiPhases { command: 1, address: 3 };

    fn command(board: &mut Jc3248w535, command: u8, params: &[u8]) {
        let mut tx = vec![0x02, 0x00, command, 0x00];
        tx.extend_from_slice(params);
        board.spi_transfer_phases(2, FRAMING, &tx, 0);
    }

    /// The split a full-frame refresh really produces: the header travels with the first chunk,
    /// everything after it is headerless payload in its own transactions.
    fn frame(board: &mut Jc3248w535, pixels: &[u8]) {
        let (head, tail) = pixels.split_at(pixels.len().min(6));
        let mut first = vec![0x32, 0x00, 0x2c, 0x00];
        first.extend_from_slice(head);
        board.spi_transfer_phases(2, FRAMING, &first, 0);
        if !tail.is_empty() { board.spi_transfer_phases(2, SpiPhases::default(), tail, 0); }
    }

    #[test]
    fn board_is_selectable_by_name() {
        assert_eq!(crate::board::make_board("jc3248w535").unwrap().name(), "jc3248w535");
        assert_eq!(crate::board::make_board("jc3248").unwrap().name(), "jc3248w535");
        assert!(crate::board::make_board("jc3248w535-v2").is_none());
    }

    #[test]
    fn a_split_qspi_frame_reaches_the_display() {
        let mut board = Jc3248w535::new();
        command(&mut board, 0x11, &[]);
        let mut pixels = vec![0u8; Jc3248w535::WIDTH * Jc3248w535::HEIGHT * 2];
        for (index, pixel) in pixels.chunks_mut(2).enumerate() { pixel.copy_from_slice(&((index % 0x10000) as u16).to_be_bytes()); }
        frame(&mut board, &pixels);
        let (w, h, frame, version) = board.display().unwrap();
        assert_eq!((w, h), (320, 480));
        assert_eq!(frame.len(), 320 * 480);
        assert_eq!((frame[0], frame[1], frame[320 * 480 - 1]), (0, 1, (320 * 480 - 1) as u16));
        assert_eq!(version, 320 * 480);
        assert_eq!(board.display_frames(), 1);
    }

    #[test]
    fn the_touch_controller_sits_at_0x3b_on_i2c0_and_follows_board_touch() {
        let mut board = Jc3248w535::new();
        let mut devices = board.i2c_devices();
        assert_eq!(devices.len(), 1);
        let (bus, address, mut device) = devices.pop().unwrap();
        assert_eq!((bus, address), (0, TOUCH_ADDRESS));

        board.touch(160, 240, true);
        assert!(device.start(false));
        for byte in Axs15231Touch::COMMAND { assert!(device.write(byte)); }
        assert!(device.start(true));
        let press: Vec<u8> = (0..8).map(|_| device.read()).collect();
        assert_eq!(Axs15231Touch::coordinates(&press.try_into().unwrap()), Some((160, 240, true)));

        board.touch(160, 240, false);
        assert!(device.start(false));
        assert!(device.start(true));
        let release: Vec<u8> = (0..8).map(|_| device.read()).collect();
        assert_eq!(Axs15231Touch::coordinates(&release.try_into().unwrap()), Some((160, 240, false)));
        assert_eq!(board.touch_counters.reports.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert_eq!(board.touch_counters.reads.load(std::sync::atomic::Ordering::Relaxed), 16);
    }

    #[test]
    fn backlight_gpio_and_panel_report_are_visible() {
        let mut board = Jc3248w535::new();
        board.gpio_changes(&[(PIN_BACKLIGHT, true)]);
        assert!(board.backlight);
        command(&mut board, 0x11, &[]);
        frame(&mut board, &[0x07, 0xe0, 0x00, 0x00]);
        let report = board.report();
        assert!(report.contains("qspi panel 320x480"), "{report}");
        assert!(report.contains("1 RAMWR"), "{report}");
        assert!(report.contains("backlight true"), "{report}");
    }
}
