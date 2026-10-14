# Complete S3 PHY bring-up without a virtual AP

## Rule and scope

The S3 FE result read at 0x60006174 exposes bit 16 regardless of whether a
virtual AP is configured. This removes the dependency on AP presence from the
existing synchronous ideal-radio completion model; it does not associate a
station, fabricate frames, or change StationLink. The "IQ estimation done"
meaning is inferred from the closed PHY driver's polling and the pre-existing
model, not a documented ESP-IDF register or a silicon timing measurement.
No calibration latency/start protocol is added in this correctness fix.

Base: ab89dc543bd8aa23b2d39f2cc48348ea5a0793ea (upstream main).

## Regression and firmware controls

`cargo +1.99.0 test --release -j2 -p esp32s3 --test wifi_phy`:
new tests fail semantically on base (0 vs 65536 without AP; 4660 vs 70196
following AP removal) and pass with the fix. Writes retain all existing word
bits. Idle/associated AP state is unchanged; reads neither create AP/network
nor deliver frames. Restoring the AP gate kills both peripheral tests and the
external no-AP firmware test; restoring the fix passes again.

`ROBOT_FACE_FW_DIR=<inputs> cargo +1.99.0 test --release -j2 -p esp32sim
--test wifi_phy -- --include-ignored`: three tests pass. Inputs are an unchanged
public Robot Face starter built with Arduino-ESP32 3.3.9 / IDF 5.5.4:

| Input | SHA-256 |
| --- | --- |
| firmware.bin | 8fc10c5ca0ddea496b733292de57a59402cdd4df8bfdfd06a625d595f7ffeda0 |
| bootloader.bin | 3a06e81d78e928687d7acd8abcd069543a4413de4398e2e2940a97b3fc739fd8 |
| partitions.bin | 1d9cca96de0fe07ad7fc0648b9878ddecd9ce565e38b589ad20fea698ed4c80c |
| esp32s3_rev0_rom.elf | c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd |

No firmware files are added; these ignored `external_` tests require explicitly
supplied public inputs and are not CI goldens. Rebuilds may produce different
bytes. Public SSID/password placeholders are test literals, not credentials.

Native CLI: `--chip s3 --boot rom --rom <rom> --bootloader <bootloader>
--ptable <partitions> --app <firmware> --flash-mb 8 --psram-mb 0 --board none
--net none --max-seconds <budget>`. Functional results, not host timing:

| AP | Base | Fix |
| --- | --- | --- |
| none | TFT-only at 10 nominal seconds | Connecting within 960,000,000 cycles (4 nominal seconds); at 20 seconds prints offline timeout and Setup complete, no connected marker, 0 received frames |
| matching public placeholders, channel 6 | Connecting, connected IP, Setup complete at 10 seconds | identical serial and guest instruction/exception/interrupt totals; AP Associated, 1 DHCP lease |
| mismatched SSID, channel 6 | Connecting and timeout dots at 10 seconds, AP Idle, no connection | identical serial and guest instruction/exception/interrupt totals; no fake link |

At the same 2,400,000,000-cycle budget, no-AP FE result reads fall from
196077903 (last value 0) to 87 (last value 0x10000). This is a register-access
count demonstrating exit from the poll, not a host CPU benchmark. The sketch
continues offline after its own association timeout, so Setup complete alone
must not be treated as evidence of connection.

## Other chips

No shared AP-gated completion path: C3 `FeIq` in esp32c3/src/wifi.rs completes
80 APB edges after its start write, without AP state; C6 `ModemBb` completes
while the IQ start bit is set; classic `FrontEnd` in esp32/src/wifi.rs uses its
own start-bit completion. None are modified. No follow-up correctness bug is
asserted for these independent paths.

## Static cost

Same release profile and toolchain; disassemble both native CLI binaries with
`xcrun llvm-objdump -d --demangle --no-show-raw-insn` and compare using address,
branch-target, alignment-padding and constant-pool normalization. All 2200
common functions are accounted for: 2198 identical, no missing functions.
Only S3 `periph_read` (808 -> 803 normalized instructions) and
`periph_write_inner` (1742 -> 1740) change. On the FE arm, the AP option load /
comparison is replaced by constant true; the result read ORs immediate 0x10000.
No added tick/deadline work, struct fields, link handling or JIT change. Raw
addresses/layout are not claimed identical. All other functions, including
scheduler/device tick/IRQ functions, have identical normalized instruction bodies.

## Required gates

All commands use `nice -n 10`, cargo +1.99.0, `-j2`, and an isolated target
outside the control checkout. Native Clippy `--workspace --all-targets --
-D warnings` and WASM Clippy `--release --target wasm32-unknown-unknown -p
esp32sim-wasm --features jit-tests -- -D warnings` exit 0.

With HOME pointing at an empty directory and only ESP32SIM_ROM_DIR selecting
`web/wasm/fw` (RUSTUP_HOME/CARGO_HOME retained solely for toolchain access),
`cargo +1.99.0 test --release -j2 --workspace -- --include-ignored --skip
external_` passes 940 tests with zero failures/ignored. The same empty HOME
without ESP32SIM_ROM_DIR, `cargo +1.99.0 test --release -j2 --workspace`,
passes 912 tests with 49 ignored and zero failures.

`RUSTUP_TOOLCHAIN=1.99.0 CARGO_BUILD_JOBS=2 tools/wasm-build.sh` exits 0;
`node tools/wasm-test.mjs hello c3-hello c6-hello c6-energy-scan c6-contiki
c6-contiki-net c6-rpl-net panel` passes all eight demos. The build script's
hardcoded `target/` copy uses a worktree symlink to the isolated target.
JIT-specific gate is not applicable: no JIT source changes. Evidence privacy
check passes: 1809 tracked evidence files / 20 gzip files, supplemented by manual
review. No golden files changed.

## CPU comparison

PENDING central quiet-machine measurement. No timing benchmark was run here.

## Limits

This is direct native firmware/peripheral validation plus the standard WASM
smoke suite, not browser-worker, display, add-on ABI or deployment acceptance.
No production data, private firmware, session replays or hardware were used.
Existing golden files and experiment rows remain unchanged.
