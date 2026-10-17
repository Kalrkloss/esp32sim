//! Board-only qualification. No I2S, DMA or guest capture claim.
use emu_core::Bus;
use esp32::bus::SocBus;
use esp_periph::i2c::INT_NACK;
use esp_soc::picture::Picture;
use std::sync::Arc;

fn put(bus: &mut SocBus, addr: u32, value: u32) { bus.write32(addr, value).unwrap(); }
fn board() -> SocBus {
    let mut bus = SocBus::new(4 << 20, [0; 6]);
    bus.board = esp32::board::make_board("ai-thinker-cam").unwrap();
    bus.attach_board_devices();
    // IDF v5.5.4 io_mux_reg.h: GPIO32 offset 0x1c; gpio_reg.h:
    // ENABLE1_W1TS at 0x30 and OUT1 at 0x10. PWDN is active high.
    put(&mut bus, 0x3ff4_901c, 2 << 12);
    put(&mut bus, 0x3ff4_4030, 1);
    route(&mut bus, 26, 27);
    bus
}
fn route(bus: &mut SocBus, sda: u8, scl: u8) {
    // IDF v5.5.4 gpio_sig_map.h I2CEXT0 29/30; io_mux_reg.h pad offsets.
    for (pin, signal) in [(sda, 30), (scl, 29)] {
        let mux = match pin { 26 => 0x28, 27 => 0x2c, _ => unreachable!() };
        put(bus, 0x3ff4_9000 + mux, (2 << 12) | (1 << 9) | (1 << 8));
        put(bus, 0x3ff4_4130 + 4 * signal, (1 << 7) | u32::from(pin));
        put(bus, 0x3ff4_4530 + 4 * u32::from(pin), signal);
    }
}
fn transfer(bus: &mut SocBus, bytes: &[u8], read: bool) -> (u32, u8) {
    let base = 0x3ff5_3000;
    put(bus, base + 0x24, u32::MAX);
    put(bus, base + 0x18, 3 << 12);
    put(bus, base + 0x1c, if read { 0x61 } else { 0x60 });
    for &b in bytes { put(bus, base + 0x1c, u32::from(b)); }
    let mut commands = vec![0, (1 << 11) | (1 << 8) | (1 + bytes.len() as u32)];
    if read { commands.push((2 << 11) | (1 << 10) | 1); }
    commands.push(3 << 11);
    for (i, cmd) in commands.into_iter().enumerate() { put(bus, base + 0x58 + 4 * i as u32, cmd); }
    put(bus, base + 4, 1 << 5);
    (bus.read32(base + 0x20).unwrap(), bus.read32(base + 0x1c).unwrap() as u8)
}
fn write(bus: &mut SocBus, reg: u8, value: u8) { assert_eq!(transfer(bus, &[reg, value], false).0 & INT_NACK, 0); }
fn read(bus: &mut SocBus, reg: u8) -> u8 {
    assert_eq!(transfer(bus, &[reg], false).0 & INT_NACK, 0);
    let (status, byte) = transfer(bus, &[], true);
    assert_eq!(status & INT_NACK, 0); byte
}
fn configure(bus: &mut SocBus) {
    // ov2640.c:set_window; geometry is in units of four pixels.
    write(bus, 0xff, 0);
    for (reg, value) in [(0x5a, 1), (0x5b, 1), (0x5c, 0), (0xda, 8), (0xe0, 0)] { write(bus, reg, value); }
}
fn clock(bus: &mut SocBus) {
    // IDF v5.5.4 ledc_reg.h / gpio_sig_map.h: HS channel 0 signal 71.
    // 80MHz / (256/256) / 2^3 = 10MHz; duty 4/8.
    put(bus, 0x3ff0_00c0, 1 << 11);
    put(bus, 0x3ff5_9140, 3 | (256 << 5) | (1 << 25));
    put(bus, 0x3ff5_9008, 4 << 4);
    put(bus, 0x3ff5_900c, (1 << 31) | (1 << 30) | (1 << 20) | (1 << 10));
    put(bus, 0x3ff5_9000, 4);
    put(bus, 0x3ff4_9044, (2 << 12) | (1 << 9));
    put(bus, 0x3ff4_4530, 71);
    // Duty becomes active at a timer boundary; no per-PCLK board stepping.
    bus.periph.tick(64);
}
fn picture(rgb: [u8; 3]) -> Picture { Picture { w: 1, h: 1, rgb: rgb.to_vec() } }

#[test]
fn aithinker_selection_and_only_sensor_attachment() {
    for alias in ["ai-thinker-cam", "esp32cam"] {
        let mut board = esp32::board::make_board(alias).unwrap();
        assert_eq!(board.name(), "ai-thinker-cam");
        let devices = board.i2c_devices();
        assert_eq!(devices.len(), 1);
        assert_eq!((devices[0].0, devices[0].1, devices[0].2.pins()), (0, 0x30, Some((26, 27))));
    }
    assert_eq!(esp32::board::aithinker::PIN_RESET, None);
    assert!(esp32::board::make_board("waveshare-cam").is_none());
}
#[test]
fn aithinker_sccb_requires_both_physical_wires() {
    let mut bus = board();
    write(&mut bus, 0xff, 1);
    assert_eq!(read(&mut bus, 0x0a), 0x26);
    route(&mut bus, 27, 26);
    assert_ne!(transfer(&mut bus, &[0x0a], false).0 & INT_NACK, 0);
    route(&mut bus, 26, 27);
    put(&mut bus, 0x3ff4_4598, 256); // disconnect SDA output only
    assert_ne!(transfer(&mut bus, &[0x0a], false).0 & INT_NACK, 0);
    route(&mut bus, 26, 27);
    put(&mut bus, 0x3ff4_459c, 256); // disconnect SCL output only
    assert_ne!(transfer(&mut bus, &[0x0a], false).0 & INT_NACK, 0);
    route(&mut bus, 26, 27);
    assert_eq!(read(&mut bus, 0x0a), 0x26);
}
#[test]
fn aithinker_host_and_register_updates_invalidate_shared_cache() {
    let mut bus = board(); configure(&mut bus); clock(&mut bus);
    assert_eq!(*bus.camera_frame().unwrap().2, [0; 32]);
    bus.board.set_camera_picture(picture([255, 0, 0])).unwrap();
    assert_eq!(bus.board.camera_preview(2, 1).unwrap(), [255, 0, 0, 255, 0, 0]);
    let first = bus.camera_frame().unwrap().2;
    assert_eq!(*first, [248, 0].repeat(16));
    assert!(Arc::ptr_eq(&first, &bus.camera_frame().unwrap().2));
    assert!(bus.board.set_camera_picture(Picture { w: 1, h: 1, rgb: vec![] }).is_err());
    assert!(Arc::ptr_eq(&first, &bus.camera_frame().unwrap().2));
    bus.board.set_camera_picture(picture([0, 255, 0])).unwrap();
    assert_eq!(*bus.camera_frame().unwrap().2, [7, 224].repeat(16));
    write(&mut bus, 0x5a, 2);
    let (w, h, frame) = bus.camera_frame().unwrap();
    assert_eq!((w, h, frame.len()), (8, 4, 64));
    write(&mut bus, 0xda, 0x10);
    assert!(bus.camera_frame().is_none(), "JPEG is not fabricated as raw bytes");
    write(&mut bus, 0xda, 8);
    assert_eq!(*bus.camera_frame().unwrap().2, [7, 224].repeat(32));
}
#[test]
fn aithinker_grayscale_requests_keep_yuv_wire_bytes() {
    let mut bus = board(); configure(&mut bus); clock(&mut bus);
    bus.board.set_camera_picture(Picture { w: 2, h: 1, rgb: vec![255, 0, 0, 0, 255, 0] }).unwrap();
    // Public grayscale and YUV422 register sequences both select IMAGE_MODE=0.
    write(&mut bus, 0xda, 0);
    assert_eq!(*bus.camera_frame().unwrap().2, [76, 84, 76, 255, 149, 43, 149, 21].repeat(4));
}
#[test]
fn aithinker_pwdn_stop_and_reset_withhold_frames() {
    let mut bus = board(); configure(&mut bus); clock(&mut bus);
    assert!(bus.camera_frame().is_some());
    put(&mut bus, 0x3ff4_4010, 1); // PWDN high
    assert!(bus.camera_frame().is_none());
    assert_ne!(transfer(&mut bus, &[0xff, 0], false).0 & INT_NACK, 0);
    put(&mut bus, 0x3ff4_4010, 0);
    assert!(bus.camera_frame().is_some());
    put(&mut bus, 0x3ff4_4034, 1); // release low PWDN
    assert!(bus.camera_frame().is_none());
    put(&mut bus, 0x3ff4_4030, 1);
    assert!(bus.camera_frame().is_some());
    for (bank, reg, value) in [(1, 9, 0x10), (0, 0xe0, 4), (0, 5, 1)] {
        write(&mut bus, 0xff, bank); write(&mut bus, reg, value);
        assert!(bus.camera_frame().is_none());
        write(&mut bus, reg, 0); assert!(bus.camera_frame().is_some());
    }
    write(&mut bus, 0xff, 1); write(&mut bus, 0x12, 0x80);
    assert!(bus.camera_frame().is_none());
    assert_eq!(read(&mut bus, 0x0a), 0x26);
}
#[test]
fn aithinker_xclk_rejects_constant_or_invalid_clock() {
    let mut bus = board(); configure(&mut bus);
    assert!(bus.board.camera_frame().is_none(), "clockless board hook cannot bypass routing");
    for hz in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(bus.board.camera_frame_with_clock(&|_| Some((hz, 32768))).is_none());
    }
    for duty in [0, 65535, 65536] {
        assert!(bus.board.camera_frame_with_clock(&|_| Some((10_000_000.0, duty))).is_none());
    }
    assert!(bus.board.camera_frame_with_clock(&|pin| {
        assert_eq!(pin, 0); Some((10_000_000.0, 32768))
    }).is_some());
}
#[test]
fn aithinker_xclk_requires_live_routed_pwm_not_gpio_level() {
    let mut bus = board(); configure(&mut bus);
    assert!(bus.camera_frame().is_none());
    clock(&mut bus);
    let (hz, duty) = bus.periph.pwm_output(0).unwrap();
    assert_eq!((hz, duty), (10_000_000.0, 32768));
    assert!(bus.camera_frame().is_some());
    put(&mut bus, 0x3ff4_4530, 256); // XCLK disconnected, GPIO high is not a clock
    put(&mut bus, 0x3ff4_4024, 1); put(&mut bus, 0x3ff4_4008, 1);
    assert!(bus.camera_frame().is_none());
    put(&mut bus, 0x3ff4_4530, 71 | (1 << 9));
    assert!(bus.camera_frame().is_some(), "inverted PWM still clocks; GPIO0 is not a reset wire");
    put(&mut bus, 0x3ff5_9140, 3 | (256 << 5) | (1 << 25) | (1 << 23)); // timer pause
    assert!(bus.camera_frame().is_none());
    clock(&mut bus);
    put(&mut bus, 0x3ff0_00c0, 0);
    assert!(bus.camera_frame().is_none());
}
