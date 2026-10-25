//! A MIPI DCS panel behind a quad-SPI link (the AXS15231B and its kin).
//!
//! In QSPI mode there is no D/C line: every transaction starts with an opcode and a 24-bit
//! address phase, and the DCS command travels inside that word. The driver encodes
//! `(opcode << 24) | (dcs_command << 8)`, so the wire bytes are
//!
//! ```text
//! 02 00 2A 00  <4 parameters>   0x02 = write command, address word 0x002A00 -> CASET
//! 02 00 2C 00                   0x2C without parameters: RAMWR, address counter from (0, 0)
//! 32 00 2C 00  <RGB565 pixels>  0x32 = write memory, address word 0x002C00 -> RAMWR
//! 32 00 3C 00  <RGB565 pixels>  0x3C = RAMWRC, a continuation of the previous memory write
//! ```
//!
//! Two shapes reach the panel, depending on how the driver programs the SPI controller:
//!
//! * the command word is the transfer's **command and address phase** (`phases.command` plus
//!   `phases.address` bytes), with parameters or pixels following in the same transfer;
//! * or the command word is a **standalone four-byte data transfer** and the parameters or pixels
//!   come as separate transactions — what ESP-IDF's `esp_lcd_panel_io_spi` does with a 32-bit
//!   `lcd_cmd_bits`: it pushes the reversed command bytes as a plain data phase and then the
//!   payload. That is the shape a MicroPython `lcd_bus` driver produces.
//!
//! Long writes are split by the DMA path: the first payload transaction follows the command word,
//! the rest are pure payload. Such a chunk must not be parsed as a header, which is why a header
//! is either delimited by the controller's phases or is exactly four bytes long — a pixel chunk of
//! that size that happens to start with an opcode byte is the one ambiguous case.
//!
//! The command set behind the framing is the DCS one, so the panel state (window, MADCTL,
//! COLMOD, inversion, sleep, display on) lives in [`DcsPanel`] and is shared with the
//! D/C-line panels.
use crate::board::SpiPhases;
use crate::devices::dcs_panel::DcsPanel;

/// Opcode of a command write; its address word carries the DCS command in bits 8..15.
pub const OPCODE_WRITE_COMMAND: u8 = 0x02;
/// Opcode of a memory write; its address word carries RAMWR (0x2C) or RAMWRC (0x3C).
pub const OPCODE_WRITE_COLOR: u8 = 0x32;
/// Bytes of a command word: opcode, two address bytes, and a zero low byte.
pub const COMMAND_WORD: usize = 4;

/// A QSPI display controller with a DCS command set and RGB565 frame memory.
pub struct QspiPanel {
    /// Command state and GRAM; the framing below only decides what reaches this.
    pub panel: DcsPanel,
    /// The AXS15231B quirk some batches show: window-address commands (CASET/RASET/PTLAR) are
    /// ignored, so every memory write starts at (0, 0) and has to carry a full frame.
    pub ignore_window: bool,
    /// Framed transactions seen (opcode plus address phase).
    pub packets: u64,
    /// DCS commands carried inside a frame.
    pub commands: u64,
    /// Payload bytes that arrived without a header (DMA continuations of a long write).
    pub continuation_bytes: u64,
    /// Transactions whose opcode is not a write opcode: reads, or a driver encoding this model
    /// does not know. Counted so a run can show that the wire format was understood.
    pub unknown_packets: u64,
}

impl QspiPanel {
    /// A controller with `cols` × `rows` of RAM, powered up (asleep, display off, 16-bit colour
    /// as the QSPI drivers set it, the window over the whole RAM).
    pub fn new(cols: usize, rows: usize) -> Self {
        QspiPanel { panel: DcsPanel::new(cols, rows), ignore_window: false, packets: 0, commands: 0, continuation_bytes: 0, unknown_packets: 0 }
    }

    /// The 3.5" 320×480 module the AXS15231B drives.
    pub fn axs15231b() -> Self { Self::new(320, 480) }

    /// Ignore window-address commands: every memory write is a full frame from (0, 0).
    pub fn ignoring_window(mut self) -> Self { self.ignore_window = true; self }

    pub fn cols(&self) -> usize { self.panel.cols }
    pub fn rows(&self) -> usize { self.panel.rows }

    /// What the glass shows, as RGB565 in panel order.
    pub fn frame(&self) -> Vec<u16> { self.panel.visible_pixels() }

    /// One complete GP-SPI transaction, with the controller's command/address phase split.
    pub fn transaction(&mut self, phases: SpiPhases, tx: &[u8]) {
        let header = phases.header();
        let framed = if phases.command != 0 && header <= tx.len() {
            // The controller delimited the header for us.
            Some(header)
        } else if phases.command == 0
            && tx.len() == COMMAND_WORD
            && matches!(tx[0], OPCODE_WRITE_COMMAND | OPCODE_WRITE_COLOR)
        {
            // A standalone command word: the driver hands the 32-bit command to the SPI
            // controller as plain data and sends the payload in the following transactions.
            Some(COMMAND_WORD)
        } else {
            None
        };
        let Some(header) = framed else {
            self.continuation_bytes += tx.len() as u64;
            self.panel.data(tx);
            return;
        };
        let word = header_word(&tx[..header]);
        self.packets += 1;
        let (opcode, command) = ((word >> 24) as u8, ((word >> 8) & 0xff) as u8);
        match opcode {
            OPCODE_WRITE_COMMAND => self.write_command(command, &tx[header..]),
            OPCODE_WRITE_COLOR => self.write_color(command, &tx[header..]),
            _ => self.unknown_packets += 1,
        }
    }

    /// A command and its parameters. The panel keeps the command until the next one, so a
    /// parameter phase split over transactions still lands (the driver sends them separately
    /// when the parameter list is longer than its transfer limit).
    fn write_command(&mut self, command: u8, params: &[u8]) {
        self.commands += 1;
        if self.ignore_window && matches!(command, 0x2a | 0x2b | 0x30 | 0x31) { return; }
        self.panel.command(command);
        self.panel.data(params);
    }

    /// Pixels (or, for a driver that leaves the address word zero, the current command's data).
    fn write_color(&mut self, command: u8, pixels: &[u8]) {
        self.commands += 1;
        match command {
            // RAMWRC continues the address counter where the previous write stopped.
            0x3c => { self.panel.continue_memory_write(); self.panel.data(pixels); }
            // RAMWR, and the IDF full-refresh path that leaves the word zero: (re)start at the
            // window origin.
            _ => {
                self.panel.command(if command == 0x00 { 0x2c } else { command });
                self.panel.data(pixels);
            }
        }
    }

    /// Lines for a board's end-of-run report.
    pub fn report(&self) -> String {
        format!("qspi panel {}x{}: {} packets ({} commands, {} unknown), {} continuation bytes, {} RAMWR, {} pixels, madctl={:#x} inverted={} on={} bbox={:?} top colours {:x?}",
                self.panel.cols, self.panel.rows, self.packets, self.commands, self.unknown_packets, self.continuation_bytes,
                self.panel.frames, self.panel.pixels_written, self.panel.madctl, self.panel.inverted, self.panel.on,
                self.panel.bbox(), self.panel.histogram(5))
    }
}

/// The header word, big-endian over the command and address phases. An 8-bit opcode followed by
/// a 24-bit address gives `opcode << 24 | address`, which is how the drivers build it.
fn header_word(header: &[u8]) -> u32 {
    let mut word = 0u32;
    for (index, &byte) in header.iter().take(4).enumerate() { word |= u32::from(byte) << (24 - 8 * index); }
    word
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMAND: SpiPhases = SpiPhases { command: 1, address: 3 };

    fn command(panel: &mut QspiPanel, command: u8, params: &[u8]) {
        let mut tx = vec![OPCODE_WRITE_COMMAND, 0, command, 0];
        tx.extend_from_slice(params);
        panel.transaction(COMMAND, &tx);
    }

    fn memory(panel: &mut QspiPanel, command: u8, pixels: &[u8]) {
        let mut tx = vec![OPCODE_WRITE_COLOR, 0, command, 0];
        tx.extend_from_slice(pixels);
        panel.transaction(COMMAND, &tx);
    }

    fn window(panel: &mut QspiPanel, x0: u16, x1: u16, y0: u16, y1: u16) {
        let mut columns = x0.to_be_bytes().to_vec();
        columns.extend_from_slice(&x1.to_be_bytes());
        command(panel, 0x2a, &columns);
        let mut rows = y0.to_be_bytes().to_vec();
        rows.extend_from_slice(&y1.to_be_bytes());
        command(panel, 0x2b, &rows);
    }

    #[test]
    fn a_full_frame_lands_row_major_at_the_origin() {
        let mut panel = QspiPanel::new(2, 2);
        memory(&mut panel, 0x2c, &[0xf8, 0x00, 0x07, 0xe0, 0x00, 0x1f, 0xff, 0xff]);
        assert_eq!(panel.panel.gram, [0xf800, 0x07e0, 0x001f, 0xffff]);
        assert_eq!((panel.panel.frames, panel.panel.pixels_written), (1, 4));
        assert_eq!(panel.packets, 1);
    }

    /// The DMA path splits a frame: the header arrives with the first chunk, the rest is pure
    /// payload. Those chunks must not be mistaken for headers, even when they start with an
    /// opcode byte.
    #[test]
    fn headerless_chunks_continue_the_write_even_when_they_start_with_an_opcode() {
        let mut panel = QspiPanel::new(4, 2);
        memory(&mut panel, 0x2c, &[0x32, 0x02]);
        panel.transaction(SpiPhases::default(), &[0x02, 0x32, 0x12, 0x34, 0x02, 0x2c]);
        assert_eq!(panel.panel.gram, [0x3202, 0x0232, 0x1234, 0x022c, 0, 0, 0, 0], "four pixels, row-major");
        assert_eq!(panel.continuation_bytes, 6);
        assert_eq!(panel.packets, 1);
    }

    #[test]
    fn ramwrc_continues_the_counter_and_both_opcodes_write_pixels() {
        let mut panel = QspiPanel::new(4, 2);
        memory(&mut panel, 0x2c, &[0x00, 0x01, 0x00, 0x02]);
        memory(&mut panel, 0x3c, &[0x00, 0x03, 0x00, 0x04]);
        assert_eq!(panel.panel.gram, [1, 2, 3, 4, 0, 0, 0, 0]);
        assert_eq!(panel.panel.frames, 1, "only RAMWR restarts a frame; RAMWRC continues it");
    }

    #[test]
    fn a_column_window_repeats_row_by_row_as_the_driver_sends_it() {
        let mut panel = QspiPanel::new(8, 4);
        window(&mut panel, 2, 5, 1, 2);
        memory(&mut panel, 0x2c, &[0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0, 8]);
        assert_eq!(panel.frame(), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 7, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    /// The quirk: a batch that ignores window commands writes every memory write from (0, 0),
    /// so a driver that sends a sub-window and then a full frame still ends up with a correct
    /// frame buffer.
    #[test]
    fn ignoring_the_window_makes_every_write_a_full_frame() {
        let mut panel = QspiPanel::new(4, 2).ignoring_window();
        window(&mut panel, 1, 2, 1, 1);
        memory(&mut panel, 0x2c, &[0, 1, 0, 2, 0, 3, 0, 4]);
        assert_eq!(panel.frame(), [1, 2, 3, 4, 0, 0, 0, 0]);
        assert_eq!(panel.panel.window(), (0, 0, 3, 1), "the window commands were dropped");
    }

    #[test]
    fn addressing_and_inversion_follow_the_dcs_commands() {
        let mut panel = QspiPanel::new(2, 2);
        command(&mut panel, 0x21, &[]);
        memory(&mut panel, 0x2c, &[0xf8, 0x00, 0x07, 0xe0, 0x00, 0x1f, 0xff, 0xff]);
        assert_eq!(panel.panel.gram[0], 0xf800);
        assert_eq!(panel.frame()[0], 0x07ff, "inversion is what the glass shows");
        command(&mut panel, 0x20, &[]);
        assert_eq!(panel.frame()[0], 0xf800);
    }

    #[test]
    fn a_parameter_command_is_not_pixels_and_sleep_out_wakes_the_panel() {
        let mut panel = QspiPanel::new(2, 2);
        command(&mut panel, 0x3a, &[0x55]);
        command(&mut panel, 0x11, &[]);
        assert!(!panel.panel.sleeping);
        assert_eq!(panel.panel.colmod, 0x55);
        assert_eq!(panel.panel.pixels_written, 0, "parameter bytes never reach GRAM");
        assert_eq!((panel.packets, panel.commands), (2, 2));
    }

    /// ESP-IDF's `esp_lcd_panel_io_spi` hands a 32-bit command to the SPI controller as plain
    /// data, so the command word is a standalone four-byte transfer and the payload follows in
    /// its own transactions. This is the shape a MicroPython `lcd_bus` driver produces.
    #[test]
    fn a_standalone_command_word_opens_the_data_stream() {
        let mut panel = QspiPanel::new(2, 2);
        panel.transaction(SpiPhases::default(), &[0x02, 0x00, 0x11, 0x00]);
        panel.transaction(SpiPhases::default(), &[0x02, 0x00, 0x2c, 0x00]);
        panel.transaction(SpiPhases::default(), &[0xf8, 0x00, 0x07, 0xe0]);
        assert_eq!(panel.panel.gram, [0xf800, 0x07e0, 0, 0]);
        assert!(!panel.panel.sleeping);
        assert_eq!((panel.packets, panel.commands, panel.continuation_bytes), (2, 2, 4));
    }

    #[test]
    fn four_byte_payloads_are_data_unless_they_look_like_a_command_word() {
        let mut panel = QspiPanel::new(2, 2);
        panel.transaction(SpiPhases::default(), &[0x32, 0x00, 0x2c, 0x00]);
        panel.transaction(SpiPhases::default(), &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(panel.panel.gram[..2], [0x1122, 0x3344]);
        assert_eq!((panel.packets, panel.continuation_bytes), (1, 4));
    }

    #[test]
    fn read_opcodes_are_counted_and_leave_the_panel_alone() {
        let mut panel = QspiPanel::new(2, 2);
        panel.transaction(COMMAND, &[0x03, 0x00, 0x04, 0x00]);
        assert_eq!((panel.packets, panel.unknown_packets, panel.commands), (1, 1, 0));
        assert_eq!(panel.panel.pixels_written, 0);
    }
}
