//! The I2C bus devices the boards hang on the controller (`esp_periph::i2c::I2c`).
pub use esp_periph::i2c::*;
use std::collections::HashMap;

pub struct Ch32v003 { pub regs: [u8; 8], ptr: u8, first: bool, pub writes: u64 }
impl Default for Ch32v003 { fn default() -> Self { Self::new() } }

impl Ch32v003 {
    pub fn new() -> Self { let mut r = [0u8; 8]; r[2] = 0xff; r[4] = 0xff; Ch32v003 { regs: r, ptr: 0, first: true, writes: 0 } }
}
impl I2cDevice for Ch32v003 {
    fn start(&mut self, read: bool) -> bool { if !read { self.first = true; } true }
    fn write(&mut self, b: u8) -> bool { if self.first { self.ptr = b & 7; self.first = false; } else { self.regs[self.ptr as usize] = b; self.writes += 1; } true }
    fn read(&mut self) -> u8 { self.regs[self.ptr as usize] }
}

/// What the board needs to know about the sensor's configuration (written over SCCB).
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorState { pub width: u32, pub height: u32, pub format: u8, pub streaming: bool, pub window: Option<[u32; 4]>, pub jpeg: bool }

/// OV5640 image sensor over SCCB: 16-bit register addresses, auto-increment.
pub struct Ov5640 { pub regs: HashMap<u16, u8>, addr: u16, phase: u8, pub writes: u64, state: std::sync::Arc<std::sync::Mutex<SensorState>> }
impl Ov5640 {
    pub fn new(state: std::sync::Arc<std::sync::Mutex<SensorState>>) -> Self {
        let mut regs = HashMap::new();
        regs.insert(0x300a, 0x56); regs.insert(0x300b, 0x40);   // chip ID 0x5640
        regs.insert(0x3008, 0x02);                              // system control: normal
        regs.insert(0x302a, 0xb0);                              // silicon revision
        let sensor = Ov5640 { regs, addr: 0, phase: 0, writes: 0, state };
        sensor.sync_state();
        sensor
    }
    pub fn get(&self, r: u16) -> u8 { *self.regs.get(&r).unwrap_or(&0) }
    fn sync_state(&self) {
        let mut st = self.state.lock().unwrap();
        st.width = ((self.get(0x3808) as u32 & 0xf) << 8) | self.get(0x3809) as u32;    // DVP output width
        st.height = ((self.get(0x380a) as u32 & 0x7) << 8) | self.get(0x380b) as u32;   // DVP output height
        st.format = self.get(0x4300);
        // esp32-camera v2.1.4 sensors/private_include/ov5640_regs.h and
        // ov5640_settings.h; format values also match esp32-camera v2.0.4.
        // Inclusive sensor-array window. Missing window registers preserve whole-image scaling.
        st.window = (0x3800..=0x3807).all(|r| self.regs.contains_key(&r)).then(|| {
            [0x3800, 0x3802, 0x3804, 0x3806].map(|r| {
                ((self.get(r) as u32 & if r & 2 == 0 { 15 } else { 7 }) << 8) | self.get(r + 1) as u32
            })
        });
        st.jpeg = self.get(0x3821) & 0x20 != 0;
        st.streaming = self.get(0x3008) & 0x40 == 0;
    }
}
impl I2cDevice for Ov5640 {
    fn start(&mut self, read: bool) -> bool { if !read { self.phase = 0; } true }
    fn write(&mut self, b: u8) -> bool {
        match self.phase {
            0 => { self.addr = (b as u16) << 8; self.phase = 1; }
            1 => { self.addr |= b as u16; self.phase = 2; }
            _ => { if self.addr == 0x3008 && b & 0x80 != 0 {
                self.regs.retain(|r, _| [0x300a, 0x300b, 0x302a].contains(r));
            }
            let v = if self.addr == 0x3008 { b & !0x80 } else { b }; self.regs.insert(self.addr, v); if (0x3800..=0x380b).contains(&self.addr) || self.addr == 0x3821 || self.addr == 0x4300 || self.addr == 0x3008 { self.sync_state(); } self.addr = self.addr.wrapping_add(1); self.writes += 1; }
        }
        true
    }
    fn read(&mut self) -> u8 { let v = self.get(self.addr); self.addr = self.addr.wrapping_add(1); v }
}

/// State of an ST7701S panel controller as seen through its 9-bit init SPI (D/C bit + 8 data bits).
#[derive(Default, Debug)]
pub struct St7701State { pub words: u64, pub last_cmd: u8, pub sleep_out: bool, pub display_on: bool, pub cmds: Vec<u8> }

/// TCA9554 / PCA9554 8-bit IO expander (regs: 0 input, 1 output, 2 polarity, 3 config). On the
/// Waveshare Touch-LCD-4B the panel's init SPI hangs off EXIO0 (CS), EXIO1 (MOSI), EXIO2 (CLK); the
/// device decodes that bit-banged stream into `St7701State`.
pub struct Tca9554 { pub regs: [u8; 4], ptr: u8, first: bool, panel: Option<std::sync::Arc<std::sync::Mutex<St7701State>>>, shift: u16, nbits: u8 }
impl Tca9554 {
    pub fn new(panel: std::sync::Arc<std::sync::Mutex<St7701State>>) -> Self { Tca9554 { regs: [0xff, 0xff, 0x00, 0xff], ptr: 0, first: true, panel: Some(panel), shift: 0, nbits: 0 } }
    /// Disconnected register-RAM initialization stub. External output effects are not modeled.
    pub fn register_ram_stub() -> Self { Tca9554 { regs: [0xff, 0xff, 0x00, 0xff], ptr: 0, first: true, panel: None, shift: 0, nbits: 0 } }
    fn input_port(&self) -> u8 { ((self.regs[1] & !self.regs[3]) | self.regs[3]) ^ self.regs[2] }
    fn output(&mut self, old: u8, new: u8) {
        let Some(panel) = self.panel.as_ref() else { return };
        let cs = new & 1 != 0; let mosi = (new >> 1) & 1; let clk_rise = new & 4 != 0 && old & 4 == 0;
        if cs { self.nbits = 0; self.shift = 0; return; }
        if clk_rise {
            self.shift = (self.shift << 1) | mosi as u16; self.nbits += 1;
            if self.nbits == 9 {
                let dc = self.shift & 0x100 != 0; let b = self.shift as u8; self.nbits = 0; self.shift = 0;
                let mut st = panel.lock().expect("ST7701 panel state mutex poisoned"); st.words += 1;
                if !dc { st.last_cmd = b; st.cmds.push(b); match b { 0x11 => st.sleep_out = true, 0x10 => st.sleep_out = false, 0x29 => st.display_on = true, 0x28 => st.display_on = false, _ => {} } }
            }
        }
    }
}
impl I2cDevice for Tca9554 {
    fn start(&mut self, read: bool) -> bool { if !read { self.first = true; } true }
    fn write(&mut self, b: u8) -> bool {
        if self.first { self.ptr = b & 3; self.first = false; }
        else if self.ptr != 0 { let old = self.regs[1]; self.regs[self.ptr as usize] = b; if self.ptr == 1 { self.output(old, b); } }
        true
    }
    fn read(&mut self) -> u8 { if self.ptr == 0 { self.input_port() } else { self.regs[self.ptr as usize] } }
}

/// Touch state shared between a board (UI) and its touch controller.
#[derive(Default, Debug, Clone, Copy)]
pub struct TouchState { pub down: bool, pub x: u16, pub y: u16, pub seen: bool, pub release_pending: bool }
impl TouchState {
    pub fn update(&mut self, x: u16, y: u16, down: bool) {
        self.x = x; self.y = y;
        if down { self.down = true; self.seen = false; self.release_pending = false; }
        else if self.seen { self.down = false; }
        else { self.release_pending = true; }
    }
    /// Keep even a short tap readable until the guest has observed the press once.
    fn observe(&mut self) {
        if self.release_pending && self.seen { self.down = false; self.release_pending = false; }
        if self.down { self.seen = true; }
    }
}

/// Goodix GT911 capacitive touch controller: 16-bit register addresses; product ID at 0x8140,
/// config at 0x8047.., status + up to 5 points at 0x814E...
pub struct Gt911 { addr: u16, phase: u8, touch: std::sync::Arc<std::sync::Mutex<TouchState>>, pub reads: u64, w: u16, h: u16 }
impl Gt911 {
    pub fn new(touch: std::sync::Arc<std::sync::Mutex<TouchState>>, w: u16, h: u16) -> Self { Gt911 { addr: 0, phase: 0, touch, reads: 0, w, h } }
    fn reg(&self, a: u16) -> u8 {
        let mut tl = self.touch.lock().unwrap();
        if a == 0x814e { tl.observe(); }
        let t = *tl;
        match a {
            0x8140 => b'9', 0x8141 => b'1', 0x8142 => b'1', 0x8143 => 0, 0x8144 => 0x60, 0x8145 => 0x10,      // "911", firmware 0x1060
            0x8047 => 0x41,                                                                                     // config version
            0x8048 => self.w as u8, 0x8049 => (self.w >> 8) as u8, 0x804a => self.h as u8, 0x804b => (self.h >> 8) as u8,
            0x804c => 5,                                                                                        // touch number
            0x814e => 0x80 | t.down as u8,                                                                      // buffer ready + count
            0x814f => 0, 0x8150 => t.x as u8, 0x8151 => (t.x >> 8) as u8, 0x8152 => t.y as u8, 0x8153 => (t.y >> 8) as u8, 0x8154 => 20, 0x8155 => 0, 0x8156 => 0,
            _ => 0,
        }
    }
}
impl I2cDevice for Gt911 {
    fn start(&mut self, read: bool) -> bool { if !read { self.phase = 0; } true }
    fn write(&mut self, b: u8) -> bool { match self.phase { 0 => { self.addr = (b as u16) << 8; self.phase = 1; } 1 => { self.addr |= b as u16; self.phase = 2; } _ => { self.addr = self.addr.wrapping_add(1); } } true }
    fn read(&mut self) -> u8 { let v = self.reg(self.addr); self.addr = self.addr.wrapping_add(1); self.reads += 1; v }
}

/// Hynitron CST820 touch controller used on the Waveshare Touch AMOLED 1.8 V2.
/// The register report matches the CST816S-compatible driver used by its board support package.
pub struct Cst820 { ptr: u8, first: bool, touch: std::sync::Arc<std::sync::Mutex<TouchState>>, pub reads: u64 }
impl Cst820 {
    pub fn new(touch: std::sync::Arc<std::sync::Mutex<TouchState>>) -> Self { Cst820 { ptr: 0, first: true, touch, reads: 0 } }
    fn reg(&self, addr: u8) -> u8 {
        let mut touch = self.touch.lock().expect("CST820 touch state mutex poisoned");
        if addr == 0x02 { touch.observe(); }
        let touch = *touch;
        match addr {
            0x01 => 0,
            0x02 => u8::from(touch.down),
            0x03 => ((touch.x >> 8) as u8) & 0x0f,
            0x04 => touch.x as u8,
            0x05 => ((touch.y >> 8) as u8) & 0x0f,
            0x06 => touch.y as u8,
            0xa7 => 0xb7,
            0xa8 => 0x41,
            0xa9 => 0x02,
            _ => 0,
        }
    }
}
impl I2cDevice for Cst820 {
    fn start(&mut self, read: bool) -> bool { if !read { self.first = true; } true }
    fn write(&mut self, byte: u8) -> bool {
        if self.first { self.ptr = byte; self.first = false; } else { self.ptr = self.ptr.wrapping_add(1); }
        true
    }
    fn read(&mut self) -> u8 { let value = self.reg(self.ptr); self.ptr = self.ptr.wrapping_add(1); self.reads += 1; value }
}

/// Hynitron CST816D capacitive touch controller (Waveshare ESP32-S3-Touch-LCD-2, address 0x15).
/// An 8-bit register pointer with auto-increment: the vendor BSP probes the chip id at 0xA7 for
/// 0xB6, then reads the finger count at 0x02 and one point at 0x03..0x06 (x and y, high nibble
/// first). 0x01 carries the gesture code the controller derives from the same pointer stream: a
/// press that travels at least `CST816_SLIDE_MIN` pixels reports that slide when it lifts,
/// anything shorter a single click.
pub const CST816_CHIP_ID: u8 = 0xb6;
pub const CST816_GESTURE_NONE: u8 = 0x00;
pub const CST816_GESTURE_SLIDE_UP: u8 = 0x01;
pub const CST816_GESTURE_SLIDE_DOWN: u8 = 0x02;
pub const CST816_GESTURE_SLIDE_LEFT: u8 = 0x03;
pub const CST816_GESTURE_SLIDE_RIGHT: u8 = 0x04;
pub const CST816_GESTURE_SINGLE_CLICK: u8 = 0x05;
/// Distance in panel pixels that separates a slide from a click.
pub const CST816_SLIDE_MIN: u16 = 40;

pub struct Cst816d {
    ptr: u8,
    first: bool,
    touch: std::sync::Arc<std::sync::Mutex<TouchState>>,
    /// where the current press started, while one is down
    press: Option<(u16, u16)>,
    gesture: u8,
    /// register reads the driver made, shared with the board for its end-of-run report
    reads: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Cst816d {
    pub fn new(touch: std::sync::Arc<std::sync::Mutex<TouchState>>, reads: std::sync::Arc<std::sync::atomic::AtomicU64>) -> Self {
        Cst816d { ptr: 0, first: true, touch, press: None, gesture: CST816_GESTURE_NONE, reads }
    }

    /// Update the gesture the way the controller does, from the point report the driver reads.
    /// A new press pins the origin; the lift turns the travel since then into a gesture.
    fn track(&mut self, state: TouchState) {
        match (state.down, self.press) {
            (true, None) => { self.press = Some((state.x, state.y)); self.gesture = CST816_GESTURE_NONE; }
            (false, Some((x0, y0))) => {
                let (dx, dy) = (state.x as i32 - x0 as i32, state.y as i32 - y0 as i32);
                self.gesture = if dx.abs().max(dy.abs()) < CST816_SLIDE_MIN as i32 {
                    CST816_GESTURE_SINGLE_CLICK
                } else if dx.abs() > dy.abs() {
                    if dx > 0 { CST816_GESTURE_SLIDE_RIGHT } else { CST816_GESTURE_SLIDE_LEFT }
                } else if dy > 0 {
                    CST816_GESTURE_SLIDE_DOWN
                } else {
                    CST816_GESTURE_SLIDE_UP
                };
                self.press = None;
            }
            _ => {}
        }
    }

    fn reg(&mut self, addr: u8) -> u8 {
        let state = {
            let mut touch = self.touch.lock().expect("CST816D touch state mutex poisoned");
            if addr == 0x02 { touch.observe(); }
            *touch
        };
        self.track(state);
        match addr {
            0x01 => self.gesture,
            0x02 => u8::from(state.down),
            0x03 => ((state.x >> 8) as u8) & 0x0f,
            0x04 => state.x as u8,
            0x05 => ((state.y >> 8) as u8) & 0x0f,
            0x06 => state.y as u8,
            0xa7 => CST816_CHIP_ID,
            0xa8 => 0x01,                  // project id
            0xa9 => 0x01,                  // firmware version
            0xec => 0x07,                  // motion mask: the gestures above are enabled
            _ => 0,
        }
    }
}

impl I2cDevice for Cst816d {
    fn start(&mut self, read: bool) -> bool { if !read { self.first = true; } true }
    fn write(&mut self, byte: u8) -> bool {
        if self.first { self.ptr = byte; self.first = false; } else { self.ptr = self.ptr.wrapping_add(1); }
        true
    }
    fn read(&mut self) -> u8 {
        let value = self.reg(self.ptr);
        self.ptr = self.ptr.wrapping_add(1);
        self.reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_registers<D: I2cDevice + ?Sized>(device: &mut D, first: u8, count: usize) -> Vec<u8> {
        assert!(device.start(false));
        assert!(device.write(first));
        assert!(device.start(true));
        (0..count).map(|_| device.read()).collect()
    }

    #[test]
    fn cst820_reports_board_identity() {
        let mut device = Cst820::new(Default::default());
        assert_eq!(read_registers(&mut device, 0xa7, 3), [0xb7, 0x41, 0x02]);
    }

    #[test]
    fn cst820_reports_touch_coordinates() {
        let touch = std::sync::Arc::new(std::sync::Mutex::new(TouchState { down: true, x: 0x167, y: 0x1bf, ..Default::default() }));
        let mut device = Cst820::new(touch);
        assert_eq!(read_registers(&mut device, 0x02, 5), [1, 0x01, 0x67, 0x01, 0xbf]);
    }

    fn cst816d_pointer(touch: &std::sync::Arc<std::sync::Mutex<TouchState>>, x: u16, y: u16, down: bool) -> Cst816d {
        {
            let mut state = touch.lock().unwrap();
            *state = TouchState::default();
            state.update(x, y, down);
        }
        Cst816d::new(touch.clone(), Default::default())
    }

    /// The vendor BSP probes 0xA7 for 0xB6, then reads the finger count and one point.
    #[test]
    fn cst816d_answers_the_vendor_probe_and_reports_one_point() {
        let touch = std::sync::Arc::new(std::sync::Mutex::new(TouchState::default()));
        let mut idle = Cst816d::new(touch.clone(), Default::default());
        assert_eq!(read_registers(&mut idle, 0xa7, 3), [0xb6, 0x01, 0x01]);
        assert_eq!(read_registers(&mut idle, 0x02, 5), [0, 0, 0, 0, 0], "no finger, no point");

        let mut device = cst816d_pointer(&touch, 0x39, 0x1a2, true);
        assert_eq!(read_registers(&mut device, 0x02, 1), [1]);
        // x = 0x039, y = 0x1a2, high nibble first
        assert_eq!(read_registers(&mut device, 0x03, 4), [0x00, 0x39, 0x01, 0xa2]);
    }

    /// A press the driver sampled and then saw lift without travel is a single click; travel of at
    /// least `CST816_SLIDE_MIN` in one axis is the slide in that direction.
    #[test]
    fn cst816d_turns_the_pointer_stream_into_gestures() {
        let touch = std::sync::Arc::new(std::sync::Mutex::new(TouchState::default()));
        let mut device = cst816d_pointer(&touch, 100, 100, true);
        assert_eq!(read_registers(&mut device, 0x02, 1), [1]);
        assert_eq!(read_registers(&mut device, 0x01, 1), [CST816_GESTURE_NONE], "still down: no gesture yet");
        { touch.lock().unwrap().update(104, 99, false); }
        assert_eq!(read_registers(&mut device, 0x02, 1), [0], "the lift");
        assert_eq!(read_registers(&mut device, 0x01, 1), [CST816_GESTURE_SINGLE_CLICK]);

        for (dx, dy, gesture) in [(0, 100, CST816_GESTURE_SLIDE_DOWN), (0, -100, CST816_GESTURE_SLIDE_UP),
                                  (100, 0, CST816_GESTURE_SLIDE_RIGHT), (-100, 0, CST816_GESTURE_SLIDE_LEFT)] {
            // a fresh controller for each gesture, as the chip starts its report clean
            let mut device = cst816d_pointer(&touch, 100, 150, true);
            assert_eq!(read_registers(&mut device, 0x02, 1), [1]);
            { touch.lock().unwrap().update((100 + dx) as u16, (150 + dy) as u16, false); }
            assert_eq!(read_registers(&mut device, 0x02, 1), [0]);
            assert_eq!(read_registers(&mut device, 0x01, 1), [gesture], "dx {dx}, dy {dy}");
        }
    }

    /// A tap shorter than the driver's poll interval — pressed and released between two reads —
    /// is reported as a press that lifts, not lost: the second read shows the lift and the click.
    #[test]
    fn cst816d_keeps_a_tap_the_driver_never_sampled() {
        let touch = std::sync::Arc::new(std::sync::Mutex::new(TouchState::default()));
        { touch.lock().unwrap().update(50, 60, true); touch.lock().unwrap().update(50, 60, false); }
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut device = Cst816d::new(touch.clone(), reads.clone());
        assert_eq!(read_registers(&mut device, 0x02, 1), [1], "the press is reported first");
        assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 1, "reads reach the board's counter");
        assert_eq!(read_registers(&mut device, 0x02, 1), [0], "then the lift the driver asked for");
        assert_eq!(read_registers(&mut device, 0x01, 1), [CST816_GESTURE_SINGLE_CLICK]);
    }

    #[test]
    fn tca9554_register_stub_keeps_written_outputs() {
        let mut device = Tca9554::register_ram_stub();
        assert!(device.start(false));
        assert!(device.write(1));
        assert!(device.write(0xa5));
        assert!(device.start(false));
        assert!(device.write(1));
        assert!(device.start(true));
        assert_eq!(device.read(), 0xa5);
    }

    #[test]
    fn tca9554_input_port_is_read_only_and_reflects_pin_levels() {
        let mut device = Tca9554::register_ram_stub();
        assert!(device.start(false));
        assert!(device.write(0));
        assert!(device.write(0));
        assert_eq!(device.regs[0], 0xff);

        assert!(device.start(false));
        assert!(device.write(3));
        assert!(device.write(0xf0));
        assert!(device.start(false));
        assert!(device.write(1));
        assert!(device.write(0x05));
        assert!(device.start(false));
        assert!(device.write(0));
        assert!(device.start(true));
        assert_eq!(device.read(), 0xf5);
    }
}

/// Board clock and motion selection shared between a board and its QMI8658.
#[derive(Default, Debug)]
pub struct ImuMotion { pub cycle: std::sync::atomic::AtomicU64, pub mode: std::sync::atomic::AtomicU32 }

/// QMI8658 IMU. Mode 0 is the register stub every existing workload was pinned against
/// (WHO_AM_I only, data never ready). Mode 1 plays a fixed 30 s handling script as a
/// function of guest time, so a benchmark that tilts and shakes the board stays exact.
/// The script produces one sample every 4 ms (250 Hz, the rate fluidbox configures; other ODR
/// settings are not modeled): timestamp and data come from the same sample, and STATUS0 reports
/// new data until a read of the sensor outputs consumes that sample.
pub struct Qmi8658 { regs: [u8; 256], ptr: u8, first: bool, consumed: Option<u32>, motion: std::sync::Arc<ImuMotion> }
impl Qmi8658 {
    pub fn new(motion: std::sync::Arc<ImuMotion>) -> Self {
        let mut regs = [0; 256];
        regs[0x00] = 0x05;
        Qmi8658 { regs, ptr: 0, first: true, consumed: None, motion }
    }
    /// Latch one sample into STATUS0 and the timestamp/temperature/accel/gyro registers.
    fn latch(&mut self, cycle: u64) {
        let stamp = (cycle / (crate::periph::CPU_HZ / 250)) as u32;
        let (accel, gyro) = motion_script(f64::from(stamp) / 250.0);
        // Scales follow the configured ranges: CTRL2[6:4] accel 2..16 g, CTRL3[6:4] gyro 16..2048 dps.
        let accel_lsb = f64::from(16384u32 >> ((self.regs[0x03] >> 4) & 3));
        let gyro_lsb = f64::from(2048u32 >> ((self.regs[0x04] >> 4) & 7));
        let quantize = |v: f64| {
            let r = if v < 0.0 { v - 0.5 } else { v + 0.5 };
            (r.clamp(-32768.0, 32767.0) as i32 as i16).to_le_bytes()
        };
        self.regs[0x2e] = if self.consumed == Some(stamp) { 0 } else { 0x03 };
        // A read of the sensor outputs consumes this sample.
        if (0x35..=0x40).contains(&self.ptr) { self.consumed = Some(stamp); }
        self.regs[0x30..0x33].copy_from_slice(&stamp.to_le_bytes()[..3]);
        self.regs[0x33..0x35].copy_from_slice(&(25i16 * 256).to_le_bytes());
        for axis in 0..3 {
            self.regs[0x35 + 2 * axis..0x37 + 2 * axis].copy_from_slice(&quantize(accel[axis] * accel_lsb));
            self.regs[0x3b + 2 * axis..0x3d + 2 * axis].copy_from_slice(&quantize(gyro[axis] * gyro_lsb));
        }
    }
}
impl I2cDevice for Qmi8658 {
    fn start(&mut self, read: bool) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        if !read { self.first = true; } else if self.motion.mode.load(Relaxed) != 0 { self.latch(self.motion.cycle.load(Relaxed)); }
        true
    }
    fn write(&mut self, b: u8) -> bool { if self.first { self.ptr = b; self.first = false; } else { self.regs[self.ptr as usize] = b; self.ptr = self.ptr.wrapping_add(1); } true }
    fn read(&mut self) -> u8 { let v = self.regs[self.ptr as usize]; self.ptr = self.ptr.wrapping_add(1); v }
}

/// Sine from + - * / and floor only, so every platform and build produces the same bits.
fn sin(x: f64) -> f64 {
    const TAU: f64 = std::f64::consts::TAU;
    let mut x = x - TAU * (x / TAU + 0.5).floor();
    if x > std::f64::consts::FRAC_PI_2 { x = std::f64::consts::PI - x; }
    if x < -std::f64::consts::FRAC_PI_2 { x = -std::f64::consts::PI - x; }
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0 * (1.0 - x2 / 110.0)))))
}
fn smoothstep(a: f64, b: f64, t: f64) -> f64 { let x = ((t - a) / (b - a)).clamp(0.0, 1.0); x * x * (3.0 - 2.0 * x) }

/// The handling script in the board's screen frame (x right, y down the screen, z into the
/// case): settle upright, then swing and lean, turn upside down and back, with three shakes.
/// Returns (accelerometer g, gyro dps) in QMI8658 axes, as the AMOLED 1.8 mounts it.
fn motion_script(t: f64) -> ([f64; 3], [f64; 3]) {
    use std::f64::consts::{PI, TAU};
    let angles = |t: f64| {
        let on = smoothstep(3.0, 5.0, t);
        let flip = PI * (smoothstep(13.0, 15.0, t) - smoothstep(20.0, 22.0, t));
        let roll = on * (1.1 * sin(TAU * (t - 3.0) / 8.0) + 0.35 * sin(TAU * (t - 3.0) / 2.9)) + flip;
        let lean = on * 0.45 * sin(TAU * (t - 3.0) / 11.0);
        (roll, lean)
    };
    let (roll, lean) = angles(t);
    let cos = |x: f64| sin(x + PI / 2.0);
    // The accelerometer reports the reaction to gravity: minus the unit "down" vector.
    let mut a = [-sin(roll) * cos(lean), -cos(roll) * cos(lean), -sin(lean)];
    for (start, axis) in [(10.0, 0), (18.0, 1), (25.0, 2)] {
        if (start..start + 0.8).contains(&t) {
            let window = sin(PI * (t - start) / 0.8);
            a[axis] += 2.0 * window * window * sin(TAU * 7.0 * (t - start));
        }
    }
    let h = 0.001;
    let ((r0, l0), (r1, l1)) = (angles(t - h), angles(t + h));
    let deg = 180.0 / PI / (2.0 * h);
    let w = [(l1 - l0) * deg, 0.0, (r1 - r0) * deg];
    // Screen frame to QMI8658 axes (inverse of fluidbox's IMU_MAP): imu = (-y, x, z).
    ([-a[1], a[0], a[2]], [-w[1], w[0], w[2]])
}

#[cfg(test)]
mod qmi8658_tests {
    use super::*;
    use std::sync::{atomic::Ordering::Relaxed, Arc};

    fn read(imu: &mut Qmi8658, reg: u8, n: usize) -> Vec<u8> {
        imu.start(false); imu.write(reg); imu.start(true);
        (0..n).map(|_| imu.read()).collect()
    }
    fn configure(imu: &mut Qmi8658) {
        for (reg, value) in [(0x03u8, 0x23u8), (0x04, 0x43)] { imu.start(false); imu.write(reg); imu.write(value); }
    }

    #[test]
    fn still_mode_is_the_old_register_stub() {
        let mut imu = Qmi8658::new(Arc::default());
        configure(&mut imu);
        assert_eq!(read(&mut imu, 0x00, 1), [0x05]);
        assert_eq!(read(&mut imu, 0x2e, 1), [0x00]);
        assert_eq!(read(&mut imu, 0x35, 12), [0; 12]);
    }

    #[test]
    fn scripted_motion_is_a_function_of_guest_time() {
        let motion = Arc::new(ImuMotion::default());
        motion.mode.store(1, Relaxed);
        let mut imu = Qmi8658::new(motion.clone());
        configure(&mut imu);
        // Upright at reset: +1 g on the QMI8658 x axis (fluidbox: "upright, port right -> (+g, 0, 0)").
        assert_eq!(read(&mut imu, 0x2e, 1), [0x03]);
        let sample = read(&mut imu, 0x35, 12);
        assert_eq!(i16::from_le_bytes([sample[0], sample[1]]), 4096);
        assert_eq!(&sample[2..], &[0; 10]);
        // Later samples tilt, keep 1 g outside shakes, and repeat bit for bit.
        let at = |imu: &mut Qmi8658, seconds: f64| {
            motion.cycle.store((seconds * crate::periph::CPU_HZ as f64) as u64, Relaxed);
            read(imu, 0x35, 12)
        };
        let tilted = at(&mut imu, 6.5);
        let g = |s: &[u8]| (0..3).map(|i| f64::from(i16::from_le_bytes([s[2 * i], s[2 * i + 1]])) / 4096.0).map(|v| v * v).sum::<f64>().sqrt();
        assert!((g(&tilted) - 1.0).abs() < 0.01 && tilted[0..2] != sample[0..2]);
        assert_eq!(at(&mut imu, 6.5), tilted);
        assert!(g(&at(&mut imu, 10.3)) > 1.5, "a shake adds linear acceleration");
        assert!((sin(1.0) - 1f64.sin()).abs() < 1e-7 && (sin(-4.0) - (-4f64).sin()).abs() < 1e-7);
    }

    #[test]
    fn one_timestamp_is_one_sample_and_is_consumed_once() {
        let motion = Arc::new(ImuMotion::default());
        motion.mode.store(1, Relaxed);
        let mut imu = Qmi8658::new(motion.clone());
        configure(&mut imu);
        let at = |imu: &mut Qmi8658, seconds: f64, reg: u8, n: usize| {
            motion.cycle.store((seconds * crate::periph::CPU_HZ as f64) as u64, Relaxed);
            read(imu, reg, n)
        };
        // Reviewer case: two reads inside one 4 ms sample during a shake see the same stamp and data.
        let (a, b) = (at(&mut imu, 10.3001, 0x30, 15), at(&mut imu, 10.3011, 0x30, 15));
        assert_eq!(a, b);
        let c = at(&mut imu, 10.3041, 0x30, 15);
        assert_ne!(a[..3], c[..3]);
        assert_ne!(a[5..], c[5..]);
        // STATUS0: new until the outputs of that sample are read, then new again at the next sample.
        assert_eq!(at(&mut imu, 12.0001, 0x2e, 1), [0x03]);
        at(&mut imu, 12.0002, 0x35, 12);
        assert_eq!(at(&mut imu, 12.0003, 0x2e, 1), [0x00]);
        assert_eq!(at(&mut imu, 12.0041, 0x2e, 1), [0x03]);
    }
}
