# Classic I2C physical routes and control layout

Base: upstream `ab89dc543bd8aa23b2d39f2cc48348ea5a0793ea`.

The classic MMIO START boundary now supplies SDA/SCL through the existing
`ChipPins::ESP32` route decoder and `I2c::set_pins`. It retains the existing
line-high timeout check and shared address matching. No fields, tick work,
command timing, runtime address or multicast rules are added or replaced.

Classic control bit 10 is not interpreted as the modern FSM reset. ESP-IDF
v5.5.4 `components/soc/esp32/register/soc/i2c_reg.h` defines the classic CTR
register but no FSM_RST field; the ESP32-S3 file at the corresponding path
has FSM_RST at lines 111–117. Preserving unmodeled classic bit 10 in register
RAM is an inferred emulator readback policy, not a reserved-bit silicon claim.
`components/soc/esp32/include/soc/gpio_sig_map.h:73–76,200–203` defines
classic I2CEXT0/1 input/output SCL/SDA signal IDs. No hardware validation or
IDF 4.4 validation is claimed.

## Reproduction

`cli/tests/classic_i2c_routes.rs` tests both controllers, APB/AHB aliases,
same-address pin-specific devices, rerouting, either wrong wire, missing
output route, classic END-recipient preservation and bit-10 readback, and
modern pending-transfer reset cancellation.

Run the three CI-runnable tests:

```
cargo +1.99.0 test --release -j2 -p esp32sim --test classic_i2c_routes
```

With a dedicated `CARGO_TARGET_DIR` and no other build running:

```
python3 docs/evidence/classic-i2c-routes/mutate.py
```

The executable table individually removes routes, reverses wires, uses port-0
signals for port 1, restores classic reset, clears classic bit 10, and removes
modern reset. A kill requires exit 101 and the named assertion test failing;
a compile error is not a kill. Original sources are restored on exit.

The new bounded Arduino Wire fixture uses the same compiled firmware for
both classic controllers and tests two correct pin pairs plus a cross-wired
pair. It is a routing probe, not a full sensor/OLED/camera model qualification.
Build the pinned PlatformIO recipe and run the ignored external test:

```
pio run -j 2 -d docs/evidence/classic-i2c-routes/firmware
CLASSIC_I2C_ROM=<ESP32-ECO3-ROM.elf> \
CLASSIC_I2C_FIRMWARE=<PlatformIO-classic-build-directory> \
cargo +1.99.0 test --release -j2 -p esp32sim --test classic_i2c_routes \
  external_arduino_classic_i2c_routes -- --ignored --exact --nocapture
```

The expected six lines are ACK/read-21, ACK/read-18, transfer-error-4/no-byte
on each port, followed by `ROUTE:done`, without reboot. The pinned Arduino
3.3.9/IDF 5.5.4 driver reports ESP_ERR_INVALID_STATE on the wrong pair and
maps it to Wire error 4, not error 2; the register-driven test separately pins
INT_NACK. No claim of silicon Wire error-code parity is made. The exact same
binary must be used
for any baseline comparison. Firmware input hashes and observed execution
results belong in the final qualification receipt.

## CPU comparison

PENDING — measured centrally only. No timing is reported here. The branch is
not ready for publication without independent exact-head review, the posted
CPU table/flag clearance and green exact-head CI.
