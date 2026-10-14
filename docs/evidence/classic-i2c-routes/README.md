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

## Executed qualification

Rust implementation/test revision: `811b69af174e4a4f6051241ca65a4478a44956dc`.
Against the frozen base above, under Rust 1.99.0 with serialized `-j2`:

- Native workspace/all-target Clippy and release WASM jit-tests Clippy with
  `-D warnings`: exit 0.
- Empty-HOME workspace CI policy (only the absolute ROM input set):
  941 passed, 0 failed. Plain empty-HOME workspace run without ROM setting:
  913 passed, 47 ignored, 0 failed.
- `RUSTUP_TOOLCHAIN=1.99.0 tools/wasm-build.sh` and
  `node tools/wasm-test.mjs hello c3-hello c6-hello c6-energy-scan c6-contiki c6-contiki-net c6-rpl-net panel`:
  exit 0. Existing golden files are unchanged. No JIT implementation changed.
- `node tools/check-evidence-privacy.mjs`: exit 0, supplemented by manual review.
- Three focused register tests pass. All six executable mutations are killed
  by the named assertion failures (exit 101), not compilation failures.
- `pio run -j 2` builds the fixture with Arduino 3.3.9/IDF 5.5.4, platform
  tag 55.03.39 (`cbc3349061987c28bc1b48d43d473e70c5ae04ed`). The ignored
  Arduino test passes all seven output lines and zero reboots. The exact same
  firmware fails the routing assertions on the frozen base: all six probes
  report error 4 and no byte, including the four correctly wired probes.

Firmware SHA-256 (linked binaries remain external inputs, not committed here):

- bootloader.bin: `83e9239d016f18e0b90fb0cac8cbe91cd506fb14852e1b618bc2844579c2a993`
- partitions.bin: `148b959cbff1c38aa8e1d5c0ba9d612c54997b945e56a63f41223eef650653a1`
- firmware.bin: `e0671ff1c67f7acf3fc6cd363aea73b501b86a2b09de514b695ea4d0cf3a0421`

Static comparison builds `cargo +1.99.0 build --release -j2 -p esp32sim --bin esp32sim`
on both revisions and disassembles with
`llvm-objdump -d --demangle --no-show-raw-insn`. Address/alignment and linker-alias
normalization yields 2194 equal functions and six differences, none missing.
Four differences (classic tick, C3/C6 refresh_irq, C3 refresh_pin_lines) are
only constant-pool zero-offset spelling versus an explicit offset: the 16-byte
Mach-O constant payloads were checked equal, with unchanged instruction counts,
opcodes and registers. All 45 selected run/step/tick/deadline functions are
therefore identical apart from verified layout-only constant references.
The two semantic differences are the intended classic I2c<16>::write control
arm and classic SocBus::store, with 37 additional static instructions and
register-allocation changes around its START route selection. MMIO stores are
not claimed instruction-identical. No fields or tick operations were added.
This is static qualification, not a CPU timing result.

Limits: this probe does not qualify complete sensor, RTC/OLED or camera/SCCB
models or establish hardware error-code parity. Modern shared addressing and
transfer timing remain covered by the unchanged workspace regressions.

## CPU comparison

PENDING — measured centrally only. No timing is reported here. The branch is
not ready for publication without independent exact-head review, the posted
CPU table/flag clearance and green exact-head CI.
