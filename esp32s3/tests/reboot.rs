//! A guest reset re-initialises every digital peripheral, so the peripheral set is rebuilt. The
//! host's own state inside it — the log an observer wants — has to cross that rebuild: `--regstat`
//! reads it, and a probe that stops recording at the guest's first reset reports a driver as idle
//! from then on. That failure mode looks exactly like firmware that configures a peripheral and
//! never uses it, so it is worth a test of its own.
use emu_core::bus::Bus;
use esp32s3::bus::SocBus;
use esp32s3::Machine;

const SPI2: u32 = 0x6002_4000;

#[test]
fn guest_reset_keeps_the_observer_register_log() {
    let mut m = Machine::new([0x02; 6], SocBus::new(16 << 20, 8 << 20, [0x02; 6]));
    // `add_observer` arms this for any observer that wants MMIO; arming it directly is the same state.
    m.bus.periph.misc.mmio_log = Some(Vec::new());
    m.bus.write32(SPI2 + 0x10, 1 << 27).unwrap();
    m.reboot();
    assert!(m.bus.periph.misc.mmio_log.is_some(), "the register log survives the reset");
    m.bus.write32(SPI2 + 0x1c, 7).unwrap();
    let log = m.bus.periph.misc.mmio_log.as_ref().expect("still armed");
    assert_eq!(log.len(), 2, "the run's history is kept and the next access is added to it");
    assert_eq!((log[1].1, log[1].3), (SPI2 + 0x1c, true), "that access is the one after the reset");
}
