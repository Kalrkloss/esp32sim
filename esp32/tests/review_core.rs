use esp_soc::SocBus as _;

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
