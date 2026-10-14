# Selected-source inspection for I2S RX

Base: upstream main `0730659d2e082949c1761c28ef59b0e1ea6268ad`.
This extends the existing EX214/EX215 and EX229 source selection, not the
PCM transport, clock model, or DMA walker.

## Contract

`I2s::rx_source() -> Option<usize>` is a read-only, non-allocating query for
the routed source slot chosen by the last receive interval, while RX is
enabled. It is not a DMA-progress or signal-quality indication. Underrun
still attributes silence to the selected source. Source/route changes
are observed on the next receive interval, not by the query. Per-controller
input, unmatched/absent banks, and chip/peripheral reset report None.
A zero-frame supported interval still selects a slot. Stopping RX makes
the query return None immediately; enabling it without reception does not
perform a new route poll. Unsupported formats retain the existing receiver
behavior; this API does not certify support for a format.

The slot is stored as zero/slot+1 in the already lazy RxState, not I2s or any bus. A
matched interval only records it in existing state after rx_data, so a
routed-but-disabled fresh receiver still allocates nothing. No policy,
register rule, hot-struct field, bus callback, or per-tick hook is added.
The existing lowest-slot selection and per-controller fallback remain.

## Regression fixture

The deterministic in-tree fixtures use two receivers, source slots 2 and 15,
distinct signed PCM16 values, 8 kHz configured RX, and matrix clock routes.
They assert output bytes, rate, attribution, source removal, clock/inversion
changes, stop, peripheral reset, chip reset, zero-frame intervals and legacy
fallback. Repeated inspection preserves queue length and source time.
A PDM fixture checks the RX WS/PDM clock, disabled/no-allocation behavior,
4 kHz converted RX, and wrong-clock attribution. Existing source/resampling
and golden firmware tests continue to exercise byte-level receive behavior.

Run targeted fixtures:

```sh
cargo +1.99.0 test -j2 --release -p esp-periph i2s::
python3 docs/evidence/pcm-selected-source/mutate.py mutations.json
```

Each mutation must fail a test assertion; compile errors are not kills.
The script restores original source in a finally block.

## Executed checks

Toolchain: rustc 1.99.0 (b940084d7 2026-09-28), native aarch64-apple-darwin
and wasm32-unknown-unknown. Cargo jobs = 2; builds use nice -n 10 and a
dedicated CARGO_TARGET_DIR. Tests use an empty HOME while preserving
CARGO_HOME/RUSTUP_HOME for the installed toolchain. No ESP32SIM environment
keys are inherited; the CI-policy run sets only ESP32SIM_ROM_DIR.

| Command | Result |
| --- | --- |
| `cargo +1.99.0 clippy -j2 --workspace --all-targets -- -D warnings` | pass |
| `cargo +1.99.0 clippy -j2 --release --target wasm32-unknown-unknown -p esp32sim-wasm --features jit-tests -- -D warnings` | pass |
| `ESP32SIM_ROM_DIR=$SOURCE/web/wasm/fw cargo +1.99.0 test -j2 --release --workspace -- --include-ignored --skip external_` | 976 passed, 0 failed |
| `cargo +1.99.0 test -j2 --release --workspace` without ROM setting | 948 passed, 47 ignored, 0 failed |
| `RUSTUP_TOOLCHAIN=1.99.0 tools/wasm-build.sh` | pass |
| `node tools/wasm-test.mjs hello c3-hello c6-hello c6-energy-scan c6-contiki c6-contiki-net c6-rpl-net panel` | all eight pass |
| `python3 docs/evidence/pcm-selected-source/mutate.py mutations.json` | 8/8 killed by assertions |
| `cargo +1.99.0 test -j2 --release -p esp-periph` after restoring mutations | pass |
| `node tools/check-evidence-privacy.mjs` | pass |

`tools/fetch-demo-assets.sh --no-linux` supplied the ROMs in this worktree.
Because wasm-build.sh copies from literal target/, target is a symlink to the
notes build directory; CARGO_TARGET_DIR remains outside the worktree.
Goldens are unchanged. No JIT source changes; wasm-jit-test.sh is not applicable.
The classic fixture checks both ports' attribution, wrong wiring and chip reset;
the S3/C3/C6 DMA fixture checks slot 15, output, rate, stop, reset and mismatch.
No hardware capture or downstream application/ABI qualification is claimed.

## Structural comparison

Built base and candidate with `cargo +1.99.0 build -j2 --release --bins` in
separate worktrees/target directories. For each of esp32sim, esp32sim-c3 and
esp32sim-c6, ran llvm-objdump `-d --demangle --no-show-raw-insn`, then:

```sh
python3 docs/evidence/pcm-selected-source/compare.py MAIN.dis BRANCH.dis
```

Each binary has 2,206 normalized identical functions, eight differences and no
added/deleted linked functions. The query is unreferenced by these CLIs.
The normalizer removes linker addresses, symbol alias choices, data page offsets
(including implicit zero offsets), alignment and no-return padding, not struct
field offsets, registers, opcodes or instruction order. This is a structural
comparison, not a claim of whole-binary byte identity or CPU equivalence.

All 74 selected run/step/tick/pending-work/pump/deadline functions are
normalized-identical, including the bus tick and I2S DMA pumps. This comparison
includes main's merged classic I2C routing; no I2C or bus-store function differs.
The eight changed functions are receive (1,050 to 1,059 instructions), shared
and classic register writes (reset clearing, plus compiler simplification of
unchanged TX-width arithmetic in the shared writer), keep_rx_input (reset clearing),
and four prepare specializations (cold allocation size 136 to 144 bytes).
The cold input/buffer/source-bank offsets and hot I2s receiver-pointer layout
are preserved. New receive work only clears/records cold attribution at existing
RX intervals; idle per-tick gates are unchanged. stats.json binds source, inputs,
artifacts and the mutation/disassembly summaries.

## CPU comparison

PENDING

No timing was run. The CPU table remains pending under the quiet paired
protocol. Exact-head review, clean mergeability and green CI gate readiness;
the pending table must be disclosed in the PR body. No performance claim
follows from structural comparisons or golden tests.
