use esp32c6::periph::OptionalPeripherals;

#[test]
fn optional_devices_preserve_the_existing_i2s_indirection() {
    assert_eq!(std::mem::offset_of!(OptionalPeripherals, i2s0), 0);
}

// Codegen contract for the native target used by the idle CPU comparison.
// Other targets still exercise the architecture-independent offset assertion above.
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
#[test]
fn cold_devices_do_not_grow_the_native_hot_bus() {
    use esp32c6::{bus::SocBus, periph::Peripherals};
    assert_eq!(std::mem::size_of::<Peripherals>(), 8624);
    assert_eq!(std::mem::size_of::<SocBus>(), 10176);
    assert_eq!(std::mem::offset_of!(Peripherals, optional), 8536);
    assert_eq!(std::mem::offset_of!(SocBus, periph), 0);
    assert_eq!(std::mem::offset_of!(SocBus, sram), 8984);
    assert_eq!(std::mem::offset_of!(SocBus, cycles), 10128);
}
