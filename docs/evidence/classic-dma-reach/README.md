# Classic DMA address reach

Classic DMA uses the internal DRAM data window [0x3ffae000, 0x40000000).
The CPU's ROM-integrity backing prefix at 0x3ffadafc is not part of that
window. IRAM aliases, mask ROM, RTC memory, flash mappings, external RAM and
registers are not DMA-capable through this interface.

Sources:

- ESP-IDF v5.5.4, `components/soc/esp32/include/soc/soc.h:203-205`:
  `SOC_DMA_LOW`/`SOC_DMA_HIGH` define the DMA-accessible memory region.
- ESP-IDF v4.4.8, the same file at lines 266-268: identical bounds.
- ESP-IDF v5.5.4,
  `components/esp_hw_support/include/esp_memory_utils.h:224-227`:
  `esp_ptr_dma_capable` checks the inclusive lower and exclusive upper bounds.
- ESP-IDF v5.5.4, `components/soc/esp32/include/soc/soc.h:195-201`:
  D/IRAM has separate data/instruction windows; use the DRAM window for DMA.

The shared `bus::dma_range` checks the entire requested span and returns
DRAM backing offsets. I2S, SPI TX/RX and Wi-Fi descriptor/payload accesses
use it. SPI payloads are checked before any transfer bytes are read or
written. A word crossing the upper boundary is rejected before writeback.
The CPU memory map and its reset-integrity prefix are unchanged.

Regression-first: with the new tests and the old implementation, I2S
passed while SPI and Wi-Fi failed at 0x3ffadafc. With the shared rule,
all three pass. Tests include both ends of DRAM, the CPU-only prefix,
IRAM/ROM/RTC addresses, crossing words and address/length overflow.

Reproduce the engine tests:

    cargo +1.99.0 test --release -p esp32 dma_reach

Actual Wi-Fi payload-engine tests additionally call `wifi_step` (TX) and
`wifi_rx_fill` (RX), with valid descriptors and payloads at the CPU-only
prefix, first DRAM byte, exact upper-end extent and one-byte crossing extent.
TX pins transmitted-frame count, queue completion/control, event/IRQ state
and unchanged backing memory. RX pins the complete header/frame/FCS bytes,
descriptor writeback, cursor and completion/IRQ state; rejected transfers
leave the entire DRAM backing (including prefix and descriptor) unchanged.

Mutation checks (each exact test must fail at runtime):

| Mutation | Exact test(s) |
| --- | --- |
| Reject an empty SPI TX node's unused buffer | `bus::tests::classic_spi_dma_empty_nodes_do_not_access_a_buffer` |
| Reject an empty SPI RX node's unused buffer | `bus::tests::classic_spi_dma_empty_nodes_do_not_access_a_buffer` |
| Admit the ROM-integrity prefix in the shared rule | `bus::tests::classic_spi_dma_reach_descriptors_and_payloads`, `wifi::tests::classic_wifi_dma_reach_descriptors_and_payloads`, `i2s::tests::classic_i2s_dma_reach_reads_and_writes` |
| Reject the first valid DRAM byte in the shared rule | Same three exact tests as above |
| Omit the SPI TX DMA-window restriction | `bus::tests::classic_spi_dma_reach_descriptors_and_payloads` |
| Omit the SPI RX DMA-window restriction | `bus::tests::classic_spi_dma_reach_descriptors_and_payloads` |
| Remove whole-word writeback preflight | `bus::tests::classic_spi_dma_reach_descriptors_and_payloads` |
| Restore Wi-Fi's private prefix-inclusive rule | `wifi::tests::classic_wifi_dma_reach_descriptors_and_payloads` |
| Restore prefix-inclusive rule only at Wi-Fi TX payload call | `wifi::tests::classic_wifi_dma_reach_tx_payload_engine` |
| Restore prefix-inclusive rule only at Wi-Fi RX payload call | `wifi::tests::classic_wifi_dma_reach_rx_payload_engine` |
| Admit the SRAM1 IRAM alias at 0x400a0000 | `bus::tests::classic_dma_reach_rejects_sram1_iram_alias` |

The first eight rows explicitly enumerate the eight original mutations (12 exact
executions). The two Wi-Fi payload call-site mutations extend them to ten / 14;
the SRAM1 alias mutation extends them to eleven / 15. After replacing SPI's
CPU-map byte loops with checked DRAM slices, the two SPI omit-window mutations
use the old prefix-inclusive backing-range rule at that call site rather than
removing a now-required range value. They still independently admit the same
CPU-only prefix; a bounds-panic or compile error is not the intended kill.
See `mutate.py` and `mutations.json` for exact replacements and outcomes.

The SRAM1 test loads a valid descriptor through the CPU-accessible instruction
window at 0x400a0000, rejects descriptor/word and SPI TX/RX payload accesses there,
and pins unchanged DRAM (including descriptor writeback) and IRAM on rejection.
The SPI copy simplification retains zero-length-node behavior and the existing
per-byte code-page invalidation calls; only payload copying uses slices.

Existing I2S, SPI and Wi-Fi end-to-end tests also exercise valid transfers,
completion/error reporting and DMA buffers. The range rule derives from
public headers; no silicon validation or Wi-Fi hardware fault-bit behavior
is claimed. No changes to clocks, tick gating, scheduler or struct layout.

## Qualification

Follow-up to reviewed head `afb8575434bd6b8c6266e06f59e1cc8b1031200a`,
without rebasing its main base `ab89dc543bd8aa23b2d39f2cc48348ea5a0793ea`.
The exact qualified Rust source bytes, workload inputs, output artifacts and
check-log digests are in `hashes.json` and `input-hashes.json`; compact results
are in `checks.json`, `mutations.json`, `firmware-comparison.json` and
`static-summary.json`. Test counts are aggregate workspace totals including
doc-tests, not just the esp32 library. No golden files were regenerated.

Rust 1.99.0, native Apple silicon:

- Native workspace/all-targets Clippy and release wasm32 jit-tests Clippy,
  warnings denied: pass.
- Empty HOME, only firmware ROM location supplied: 945 passed, 0 failed,
  0 ignored, using `--include-ignored --skip external_`.
- Empty HOME, no ROM location: 917 passed, 0 failed, 46 ignored.
- Production WASM build and all eight standard demos, including JIT handoff:
  pass. There is no JIT source change requiring the separate JIT suite.
- Eleven distinct mutations / 15 exact runtime assertion failures, no compile
  or bounds-panic kills; restored-source DMA tests pass.
- Evidence privacy and whitespace checks: pass.
- Existing tracked golden and firmware bytes are unchanged relative to main.
- Classic hello, 3 s ROM boot, before/after this follow-up: identical console
  SHA-256 `8d8b7d143063e8d3d97b278cc43b326fc0378bf89d6a6ff7091a1f72b4c4c4ff`,
  interrupt-report SHA-256
  `799c4dbafde24dc1ac614e3afa9fdb491d8c24ae844bb07f4b1316ac87e14e37`,
  3,174 exceptions and 597 interrupts, including identical per-source counts.

Reproduce from the repository root (OUT is a fresh writable evidence directory):

    tools/fetch-demo-assets.sh --no-linux
    # wasm-build.sh copies from target/; point that path at the external build directory.
    mkdir -p "$OUT/target"
    ln -s "$OUT/target" target
    nice -n 10 python3 docs/evidence/classic-dma-reach/checks.py "$PWD" "$OUT"
    nice -n 10 python3 docs/evidence/classic-dma-reach/mutate.py "$PWD" "$OUT"

Run the mutation script sequentially, with no concurrent source build in that
worktree. It restores each mutated file in a finally block. Its target directory
is under OUT. If target already exists, use a fresh worktree instead of replacing
it. The empty HOME tests retain explicit RUSTUP_HOME/CARGO_HOME so Rust's installed
toolchain remains available; no firmware discovery falls back to the real HOME.

## Disassembly

Fresh release binaries compare this follow-up with `afb85754`. The normalizer
removes relocation addresses, branch targets and constant-pool page offsets;
it preserves opcodes, registers, stack/field offsets and other immediates.
The second-pass zero-page-offset classification is retained separately, not
silently folded into the first-pass counts:

| Binary | Common functions | First-pass identical | Identical including zero-page-offset relocations |
| --- | ---: | ---: | ---: |
| esp32sim | 2,199 | 2,193 | 2,198 |
| esp32sim-c3 | 2,199 | 2,196 | 2,198 |
| esp32sim-c6 | 2,199 | 2,196 | 2,198 |

The zero-page-offset-only functions are named in `static-summary.json`; their
only differences are a constant-pool load's `[xN]` versus `[xN, #PAGEOFF]`.
The only substantive common-function difference is classic `SocBus::store`:
3,166 → 2,943 normalized instructions, local stack allocation 0xc70 → 0xc20
(excluding the unchanged saved-register area). SPI command dispatch is inlined
there; the smaller body replaces generic CPU-map byte access with checked DRAM
copies. Register allocation and block placement in that function also change;
this is not a claim that every individual store path is faster.

Classic `SocBus::tick` is identical: 1,770 instructions, stack 0x1e0 and the
same nine-instruction prologue. Compared scheduler/run-loop, IRQ refresh, reads
and code-page-version functions are identical. No new idle tick work is added.
This static comparison justifies keeping the slice simplification, not a new
measured speed claim. `static-firmware.py` reproduces the comparison on macOS
with Command Line Tools llvm-objdump: put previous-head binaries in OUT/baseline
and follow-up binaries in OUT/target/release, then run it with repository root
and OUT arguments. Build both revisions with
`CARGO_TARGET_DIR="$OUT/target" nice -n 10 cargo +1.99.0 build --release --bins`,
saving the previous binaries before building the follow-up.

## CPU comparison

Measured centrally by joakimeriksson, posted October 14, 2026 in
[the #225 review](https://github.com/mikroverk/esp32sim/pull/225#issuecomment-6177864995),
main `ab89dc54` versus reviewed head `afb85754`:

| Workload | main median (s) | PR median (s) | Change | PR slower in |
| --- | ---: | ---: | ---: | ---: |
| classic hello (30 s) | 1.86 | 1.86 | +0.0% | 3 of 7 |

His stated method: user CPU, Apple silicon, 1-minute load 5–7 throughout,
7 alternating pairs, identical instruction counts and console hashes in every
pair. The comment does not supply warmup details or per-pair samples; none are
inferred here.

He also measured all eight open PRs merged on a scratch branch against main
`ab89dc54`, with the same method: classic hello +0.6% (3/7 slower), C3 hello
0.0% (3/7), C6 hello −0.6% (2/7), S3 hello −1.1% (3/7), Pocket Tank +0.2%
(3/7). Nothing was slower in six or more pairs. Those combined results do not
isolate this PR and no aggregate-branch SHA was supplied. The standalone table
and combined results are transcribed in `cpu-maintainer.json`.

These measurements predate the SPI slice follow-up; they are not relabeled as
measurements of it. No additional local CPU run or speed improvement is claimed.
Rebase onto main is deferred until #222/#224 land, as requested. Preserve both
adjacent test groups in wifi.rs/i2s.rs and keep the cfg(test) DRAM_HIGH/ROM_DRAM_LOW
imports that #222's tests require.
