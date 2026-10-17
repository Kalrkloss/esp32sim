//! Fixed AI-Thinker ESP32-CAM wiring from Arduino-ESP32 2.0.17
//! CameraWebServer/camera_pins.h (CAMERA_MODEL_AI_THINKER).
//! This is a host frame source, not an I2S/DVP capture engine.
use esp_periph::i2c::I2cDevice;
use esp_soc::{board::BoardModel, camera::{Ov2640, Ov2640State, HostFrameSource}, picture::{Picture, PixelFormat}};
use std::sync::{Arc, Mutex};

pub const PIN_PWDN: u8 = 32;
pub const PIN_XCLK: u8 = 0;
/// The module has no reset wire. Never interpret -1 as GPIO 255 or GPIO 0.
pub const PIN_RESET: Option<u8> = None;

pub struct AiThinkerCam {
    sensor: Arc<Mutex<Ov2640State>>,
    source: HostFrameSource,
    power: Arc<Mutex<bool>>,
}
impl Default for AiThinkerCam { fn default() -> Self { Self { sensor: Default::default(), source: Default::default(), power: Arc::new(Mutex::new(true)) } } }

struct Sccb { sensor: Ov2640, powered_down: Arc<Mutex<bool>> }
// Power is shared with the SCCB device so a powered-down sensor does not ACK.
// The board's output callbacks maintain this independently of frame requests.
impl I2cDevice for Sccb {
    fn pins(&self) -> Option<(u8, u8)> { Some((26, 27)) }
    fn start(&mut self, read: bool) -> bool { !*self.powered_down.lock().unwrap() && self.sensor.start(read) }
    fn write(&mut self, byte: u8) -> bool { self.sensor.write(byte) }
    fn read(&mut self) -> u8 { self.sensor.read() }
    fn stop(&mut self) { self.sensor.stop(); }
}

impl BoardModel for AiThinkerCam {
    fn name(&self) -> &'static str { "ai-thinker-cam" }
    fn gpio_output_at(&mut self, _: u64, _: &[(u8, bool)], enabled: u64, output: u64) {
        // A floating/unconfigured PWDN is conservatively inactive. The guest must
        // explicitly drive it low; no reset GPIO is attached on this board.
        *self.power.lock().unwrap() = enabled & (1 << PIN_PWDN) == 0 || output & (1 << PIN_PWDN) != 0;
    }
    fn i2c_devices(&mut self) -> Vec<(u8, u8, Box<dyn I2cDevice>)> {
        vec![(0, 0x30, Box::new(Sccb { sensor: Ov2640::new(self.sensor.clone()), powered_down: self.power.clone() }))]
    }
    fn set_camera_picture(&mut self, p: Picture) -> Result<(), &'static str> { self.source.set_camera_picture(p) }
    fn camera_preview(&self, w: u32, h: u32) -> Option<Vec<u8>> { self.source.camera_preview(w, h) }
    fn camera_frame_with_clock(&mut self, clock: &dyn Fn(u8) -> Option<(f64, u32)>) -> Option<(u32, u32, Arc<Vec<u8>>)> {
        let config = *self.sensor.lock().unwrap();
        let (hz, duty) = clock(PIN_XCLK)?;
        if *self.power.lock().unwrap() || !config.streaming || !hz.is_finite() || hz <= 0.0 || duty == 0 || duty >= 65535 { return None; }
        // esp32-camera v2.1.4 ov2640_settings.h: grayscale requests use
        // ov2640_settings_yuv422, exactly the same two-byte wire format as YUV.
        let format = match config.image_mode {
            0 => PixelFormat::Yuyv,
            8 => PixelFormat::Rgb565,
            _ => return None, // JPEG/raw/byte-order variants are not implemented.
        };
        self.source.frame(config.width, config.height, format, None)
    }
}
