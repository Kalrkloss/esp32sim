//! Scheduled classic camera transport. Uses the native I2S channel and its one
//! internal-DRAM adapter; no S3 GDMA registers or second descriptor walker.
use crate::{board::aithinker::{FRAME_CYCLES, VSYNC_CYCLES}, bus::SocBus, camera_fifo::CameraFifo};

#[derive(Default)]
pub(crate) struct Capture {
    saw_vsync: bool,
    packed: Vec<u8>,
    pos: usize,
    elapsed: u64,
}
impl SocBus {
    // Only entered behind the existing clock + running-DMA gate. Keeping this
    // out of line prevents camera packing from expanding the classic tick body.
    #[inline(never)]
    pub(crate) fn camera_dma_data(&mut self, cycles: u64) -> Option<Vec<u8>> {
        let rx = self.periph.i2s[0].camera_rx()?;
        if !rx.started { return Some(Vec::new()); }
        let Some((width, height, wire)) = self.camera_routed_frame() else {
            if self.periph.i2s[0].camera.is_some() {
                self.periph.i2s[0].abort_camera_dma();
                self.irq_dirty = true;
            }
            return Some(Vec::new());
        };
        let high = self.periph.gpio.signal_input(191) == Some(true);
        let capture = self.periph.i2s[0].camera.get_or_insert_with(Default::default);
        if high {
            // End the previous active interval at the next VSYNC. Flush its
            // final whole words before beginning a new frame. EOF is a native
            // chunk interrupt, never an invented whole-frame completion flag.
            let tail = capture.packed[capture.pos..].to_vec();
            capture.packed.clear(); capture.pos = 0; capture.elapsed = 0;
            capture.saw_vsync = true;
            return Some(tail);
        }
        if capture.packed.is_empty() {
            if !capture.saw_vsync { return Some(Vec::new()); }
            capture.saw_vsync = false;
            let Some(row_bytes) = (width as usize).checked_mul(2).filter(|n| *n != 0) else { return Some(Vec::new()) };
            if wire.len() != row_bytes.saturating_mul(height as usize) { return Some(Vec::new()); }
            let Ok(mut fifo) = CameraFifo::new(rx.fifo_mode) else { return Some(Vec::new()) };
            for row in wire.chunks_exact(row_bytes) { fifo.push(row, &mut capture.packed); fifo.end_row(&mut capture.packed); }
            // The falling edge begins the active frame. Do not charge the VSYNC
            // pulse (or a delayed ISR's previous interval) as captured pixels.
            return Some(Vec::new());
        }
        capture.elapsed = capture.elapsed.saturating_add(cycles).min(FRAME_CYCLES - VSYNC_CYCLES);
        // Approximate uniform word delivery over the existing 10Hz frame. This
        // is not PCLK timing. Whole words survive arbitrary guest tick splits.
        let words = capture.packed.len() / 4;
        let end = ((capture.elapsed as u128 * words as u128) / u128::from(FRAME_CYCLES - VSYNC_CYCLES)) as usize * 4;
        let data = capture.packed[capture.pos..end].to_vec();
        capture.pos = end;
        Some(data)
    }
}
