use esp_soc::SocBus as _;
use emu_core::bus::Bus as _;

#[test]
fn wakeup_state_cannot_alias_reset_cause() {
    let mut bus = esp32::bus::SocBus::new(4 << 20, [0; 6]);
    bus.set_reset_cause(12);
    assert_eq!(bus.read32(0x3ff4_8038).unwrap(), 12 << 11);
    bus.write32(0x3ff4_8038, u32::MAX).unwrap();
    assert_eq!(bus.read32(0x3ff4_8034).unwrap(), 12 | (12 << 6));
    assert_eq!(bus.read32(0x3ff4_8038).unwrap(), 0xfff << 11);
}

#[test]
fn gpio23_is_visible_but_unbonded_or_invalid_pins_are_hidden() {
    let bus = esp32::bus::SocBus::new(4 << 20, [0; 6]);
    assert!(bus.gpio_state(23).is_some());
    for pin in [20, 24, 28, 29, 30, 31, 40, 255] { assert!(bus.gpio_state(pin).is_none()); }
}

#[test]
fn reset_disables_classic_rwdt_without_clobbering_stage3() {
    let mut bus = esp32::bus::SocBus::new(4 << 20, [0; 6]);
    bus.write32(0x3ff4_80a4, 0x50d8_3aa1).unwrap();
    bus.write32(0x3ff4_8098, 0x1234).unwrap();
    bus.write32(0x3ff4_808c, (1 << 31) | (3 << 28)).unwrap();
    bus.request_reset(12);
    bus.reboot([0; 6]);
    assert_eq!(bus.read32(0x3ff4_808c).unwrap(), 0);
    assert_eq!(bus.read32(0x3ff4_8098).unwrap(), 0x1234);
}

#[test]
fn reset_keeps_both_flash_jedec_ids() {
    let mut bus = esp32::bus::SocBus::new(16 << 20, [0; 6]);
    bus.set_flash_size(16 << 20);
    let ids = [bus.periph.spi0.0.jedec, bus.periph.spi1.0.jedec];
    assert_eq!(ids[0][2], 24);
    for _ in 0..3 {
        bus.request_reset(12);
        bus.reboot([0; 6]);
        assert_eq!([bus.periph.spi0.0.jedec, bus.periph.spi1.0.jedec], ids);
    }
}
