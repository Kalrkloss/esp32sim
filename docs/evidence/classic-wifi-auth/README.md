# Classic Wi-Fi authentication TX lengths (EX226 follow-up)

Base: `ab89dc543bd8aa23b2d39f2cc48348ea5a0793ea` (merged upstream main).
Code: `fb3e5a660a82cc953f3a290e1cb9fce952609831`.

## Rule and limits

The classic TX adapter uses descriptor `length` as the number of frame bytes,
independently of `size`. Arduino-ESP32 3.3.8 libpp authentication descriptors
use length 30 and size 28. This is an inferred library compatibility rule, not
silicon validation or a new public MAC-register definition. No exact-value
30/28 workaround is added to the frame decoder. Ownership, the complete
12-byte descriptor extent and the entire frame's mapped-DRAM extent remain
mandatory. RX capacity checks are unchanged. Multi-descriptor frames, PSRAM DMA
and RF timing are outside this change.

The regression submits a real 30-byte open-system authentication request
through the classic MAC to the shared access point. For capacities 28, 30 and
32 it pins completion, queue control, TX count and the AP's successful
sequence-2 response. Separate negatives exercise CPU-owned descriptors,
length crossing DRAM's upper bound despite size fitting, below-DRAM and
near-u32-limit buffers, and a truncated descriptor. Invalid TX must leave
completion and queue control unpublished. No shared link, bus layout or
other-chip source is changed.

## Checks

Rust 1.99.0, serial builds with `-j2` and `nice -n 10`:

- `cargo +1.99.0 clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo +1.99.0 clippy --release --target wasm32-unknown-unknown -p esp32sim-wasm --features jit-tests -- -D warnings`: pass.
- Empty HOME, only `ESP32SIM_ROM_DIR` as emulator input:
  `cargo +1.99.0 test --release --workspace -- --include-ignored --skip external_`:
  939 passed, 0 failed, 0 ignored (includes doc tests).
- Empty HOME, no `ESP32SIM_*` input:
  `cargo +1.99.0 test --release --workspace`: 911 passed, 0 failed, 46 ignored.
- `RUSTUP_TOOLCHAIN=1.99.0 tools/wasm-build.sh`: pass.
- `node tools/wasm-test.mjs hello c3-hello c6-hello c6-energy-scan c6-contiki c6-contiki-net c6-rpl-net panel`: all eight pass.
- `node tools/check-evidence-privacy.mjs`: pass.
- `cargo +1.99.0 test --release -p esp32 --lib wifi::tests`: six pass.
- Existing goldens and `docs/experiments.md`: unchanged. JIT implementation
  unchanged; the separate JIT-change suite is not applicable.

Fetch inputs in the worktree with `tools/fetch-demo-assets.sh --no-linux`.
Keep `CARGO_HOME` and `RUSTUP_HOME` pointing to installed toolchains when
changing HOME; no firmware input is discovered from the normal home directory.
`tools/wasm-build.sh` expects `target/`, so when setting `CARGO_TARGET_DIR`
use a `target` symlink to that dedicated build directory.

## Mutations

Run `CARGO_TARGET_DIR=<build> python3 docs/evidence/classic-wifi-auth/mutate.py <results>`.
All eight compiled and failed at runtime, rather than failing compilation:

| Replacement | Killing test |
| --- | --- |
| Restore length > size rejection | authentication_tx_reads_full_length_not_buffer_capacity |
| Read size bytes, not length | authentication_tx_reads_full_length_not_buffer_capacity |
| Ignore DMA ownership | invalid_tx_descriptors_never_publish_completion |
| Bound size bytes, not length | invalid_tx_descriptors_never_publish_completion |
| Skip descriptor extent check | invalid_tx_descriptors_never_publish_completion |
| Complete before validating buffer extent | invalid_tx_descriptors_never_publish_completion |
| Omit station delivery | authentication_tx_reads_full_length_not_buffer_capacity |
| Omit TX completion | authentication_tx_reads_full_length_not_buffer_capacity |

## Static comparison

Build both frozen revisions with
`cargo +1.99.0 build --release -j2 -p esp32sim --bins`, using separate target
directories (a shared output path can retain another worktree's top-level
binary even when Cargo says fresh). Disassemble with
`xcrun llvm-objdump -d --demangle --no-show-raw-insn <esp32sim>`.

After normalizing addresses, alignment nops, data-page relocations and
identical-body linker aliases, 2,199 common functions match; one changes:
classic `SocBus::tick`, which inlines `wifi_step`, shrinks from 1,854 to 1,851
instructions. Its first 1,262 instructions match. The active TX descriptor
block loses size extraction and the length/capacity conditional comparison;
remaining differences are register allocation for the frame offset/length.
All 119 selected S3/C3/C6/core/peripheral hot functions match, with none
missing. No fields, tick calls or idle gates are added. This is a static
comparison, not a measured CPU clearance. Artifact identities are in
`static.json`.

## CPU comparison

PENDING — central quiet run only: one warmup and seven alternating pairs,
start load below 5, redo above 7, no concurrent builds, equal instructions
and console hashes. Slower in at least six pairs requires explanation.
No timing or ready-for-review performance acceptance is claimed here.
