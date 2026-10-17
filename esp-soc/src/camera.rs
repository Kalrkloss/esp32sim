//! Camera sensor and host-frame models, independent of a SoC's capture peripheral.
//! OV5640 register encodings and its 2624 x 1952 array remain sensor-specific;
//! this module does not model LCD_CAM, I2S, DMA, board wiring or capture timing.
use esp_periph::i2c::I2cDevice;
use crate::picture;
use std::collections::HashMap;

/// What the board needs to know about the sensor's configuration (written over SCCB).
/// `format` and the inclusive array `window` retain OV5640 register meanings;
/// neither is a generic pixel-format or SoC peripheral register encoding.
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

/// Shared OV5640 sensor configuration, host picture and converted-frame cache.
/// Frames contain RGB565 big-endian, YUYV, or grayscale bytes according to the
/// sensor's output registers. JPEG remains unsupported. Replacing the picture
/// or changing sensor configuration invalidates the converted frame.
#[derive(Default)]
pub struct Ov5640Camera {
    pub sensor: std::sync::Arc<std::sync::Mutex<SensorState>>,
    picture: Option<picture::Picture>,
    frame: Option<(u32, u32, std::sync::Arc<Vec<u8>>)>,
    frame_config: Option<SensorState>,
}
impl Ov5640Camera {
    pub fn set_camera_picture(&mut self, p: picture::Picture) -> Result<(), &'static str> {
        if !p.valid() { return Err("invalid camera picture or frame exceeds 8 MiB"); }
        self.picture = Some(p); self.frame = None; Ok(())
    }

    pub fn camera_preview(&self, w: u32, h: u32) -> Option<Vec<u8>> {
        let p = self.picture.as_ref()?;
        let mut out = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h { let sy = (y as u64 * p.h as u64 / h as u64) as usize; for x in 0..w { let sx = (x as u64 * p.w as u64 / w as u64) as usize; let o = (sy * p.w as usize + sx) * 3; out.extend_from_slice(&p.rgb[o..o + 3]); } }
        Some(out)
    }
    pub fn camera_frame(&mut self) -> Option<(u32, u32, std::sync::Arc<Vec<u8>>)> {
        let config = *self.sensor.lock().unwrap();
        let (w, h) = (config.width, config.height);
        if w == 0 || h == 0 || !config.streaming { return None; }
        if self.frame.is_none() || self.frame_config != Some(config) {
            let black;
            let p = match self.picture.as_ref() {
                Some(p) => p,
                None => { black = picture::Picture { w: 1, h: 1, rgb: vec![0; 3] }; &black }
            };
            // Window coordinates cover the 2624 x 1952 array in esp32-camera v2.1.4's ratio table.
            let cropped;
            let p = if let Some(window) = config.window {
                cropped = picture::crop(p, window, 2624, 1952)?;
                &cropped
            } else { p };
            // esp32-camera v2.1.4 ov5640_settings.h FORMAT_CTRL00 (0x4300).
            let format = match (config.jpeg, config.format) {
                (false, 0x61) => Some(picture::PixelFormat::Rgb565),
                (false, 0x30) if w.is_multiple_of(2) => Some(picture::PixelFormat::Yuyv),
                (false, 0x10) => Some(picture::PixelFormat::Grayscale),
                _ => None,
            };
            let Some(format) = format else {
                if self.frame_config != Some(config) { eprintln!("[camera] unsupported sensor output: format {:#x}, JPEG {}, {}x{}", config.format, config.jpeg, w, h); }
                self.frame = None; self.frame_config = Some(config); return None;
            };
            let data = picture::sensor_frame(p, w, h, format)?;
            self.frame = Some((w, h, std::sync::Arc::new(data)));
            self.frame_config = Some(config);
        }
        self.frame.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn write(sensor: &mut Ov5640, reg: u16, value: u8) {
        assert!(sensor.start(false));
        for byte in reg.to_be_bytes().into_iter().chain([value]) { assert!(sensor.write(byte)); }
        sensor.stop();
    }

    #[test]
    fn camera_shared_model_needs_no_soc_and_preserves_frame_cache() {
        let mut camera = Ov5640Camera::default();
        let mut sensor = Ov5640::new(camera.sensor.clone());
        assert!(camera.camera_frame().is_none());
        for (reg, value) in [(0x3809, 2), (0x380b, 1), (0x4300, 0x61)] { write(&mut sensor, reg, value); }
        assert_eq!(*camera.camera_frame().unwrap().2, [0; 4]);
        let picture = || picture::Picture { w: 2, h: 1, rgb: vec![255, 0, 0, 0, 255, 0] };
        camera.set_camera_picture(picture()).unwrap();
        assert_eq!(camera.camera_preview(2, 1).unwrap(), picture().rgb);
        let first = camera.camera_frame().unwrap().2;
        assert_eq!(*first, [248, 0, 7, 224]);
        assert!(Arc::ptr_eq(&first, &camera.camera_frame().unwrap().2));
        assert!(camera.set_camera_picture(picture::Picture { w: 2, h: 1, rgb: vec![] }).is_err());
        assert!(Arc::ptr_eq(&first, &camera.camera_frame().unwrap().2), "invalid input keeps the current cache");
        camera.set_camera_picture(picture()).unwrap();
        assert!(!Arc::ptr_eq(&first, &camera.camera_frame().unwrap().2), "valid replacement invalidates even an equal-size cache");
        write(&mut sensor, 0x4300, 0x10);
        assert_eq!(*camera.camera_frame().unwrap().2, [76, 149]);
        write(&mut sensor, 0x3008, 0x42);
        assert!(camera.camera_frame().is_none());
        write(&mut sensor, 0x3008, 2);
        assert_eq!(*camera.camera_frame().unwrap().2, [76, 149]);
        write(&mut sensor, 0x3821, 0x20);
        assert!(camera.camera_frame().is_none());
        write(&mut sensor, 0x3008, 0x82);
        assert_eq!((sensor.get(0x300a), sensor.get(0x300b)), (0x56, 0x40));
        assert!(camera.camera_frame().is_none());
    }
}
