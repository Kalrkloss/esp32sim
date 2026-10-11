# EX219: LED routing and DMA output

Base: upstream main `a175e357` (#203 merged; tree identical to `15655505`),
including merged #201 and #202. Branch: `led-rmt-parlio-fixes`.
The portable layout follow-up changes tests and evidence, not production code.
Related EX205 provides CPU-fed
compact RMT and board pin transport. This change adds mirrored peripheral routes, S3 TX3 DMA
symbols, and C6 parallel output. It reuses the WS2812 timing constants that EX218 (#204) added
to `esp-soc/src/devices/ws2812.rs`; the GPIO waveform decoder itself is untouched. GDMA OUT
descriptors are read through EX214's walker and EX220's `gather_out`.

## Behavior

RMT delivery iterates every valid GPIO matrix route, using main's `PinRoutes`
for IO_MUX, chip-specific masks, output inversion and output-enable selection.
Two enabled pins on the same signal each receive a frame. A disabled
software-enable route, inverted route, or non-GPIO mux function does not.

While TX3 DMA_ACCESS_EN is set, S3 stages TX3 symbols when RMT TX3 CONF0 is written,
or when a GDMA register of a running trigger-9 OUT channel is written. Staging uses
#203's shared `gather_out` over #201's descriptor walker; this branch adds no
descriptor reader. Channel 3 consumes the staged symbols through the existing timed
RMT transmitter. DMA mode bypasses CPU-fed RMT memory exhaustion and threshold
interrupts. FIFO reset and disabling DMA discard staged symbols; starvation waits for
more data.

Each staging step stops after one SUC_EOF node. A chain whose last node links to null
completes (OUT_TOTAL_EOF, channel stopped). A node with SUC_EOF and a non-null link
keeps the channel running and records where staging resumes. This is the pattern of
IDF's RMT DMA driver (v5.5.5 `components/esp_driver_rmt/src/rmt_tx.c`): two ring nodes
of half the DMA buffer each, both with SUC_EOF, owner check and automatic write-back
(`42-46`, `79-95`, `693-701`). The OUT_EOF callback re-encodes the consumed half, hands
it back to DMA and calls `gdma_append`, which sets OUT_LINK RESTART (`1105-1132`). The
last node's link is cut when the frame ends (`rmt_tx_mark_eof`, `602-649`). RESTART
resumes after the last staged node rather than at the link address. Staging is
instantaneous, so it can reach a ring node that the callback has not refilled yet,
where real hardware is still held back by the RMT FIFO. Such a CPU-owned node at the
resume point waits for the next append instead of raising a descriptor error. A test
replays this sequence for a four-half frame and checks every symbol, the owner
handback and the absence of descriptor errors.

C6 stages finite PARLIO chains on PARLIO and PCR TX-clock writes, and on GDMA
register writes of a running trigger-9 OUT channel, including prefill while the
TX clock is disabled. The PCR PARL_CLK_TX_CONF value (clock and TX reset) is applied
when staging, so a PCR write before PARLIO is used allocates nothing. SPI2, AES and
PARLIO use the same shared gather through one C6 wrapper that stops the channel after
the transaction. A configured start, complete payload and enabled clock produce lane
samples and TX EOF. Packing widths 1, 2, 4, 8 and 16, both bit orders, mirrored lanes
and interrupt enable/clear are modeled. A free function next to `Ws2812Chain`
converts lane samples to the existing board `rmt_frame` API, iterating routes
directly. It uses the WS2812 timing constants EX218 added. Completed PARLIO data
stores samples only; width and clock remain on the peripheral.

Invalid descriptors, lengths, alignment, buffer addresses and cyclic chains cannot
produce unbounded output. S3 queues at most 4096 pixels' symbols plus one end marker
(staged and not yet transmitted); C6 queues at most 65535 bytes. These are emulator
limits; a longer frame raises the RMT DMA error (S3) or a GDMA descriptor error (C6).

## Behaviour changes outside the LED path

These change paths that are not LED output. They are listed here and in the PR
description.

* **Shared OUT gather rules** (`esp-periph/src/gdma/transmit.rs`; used by S3 SHA/AES,
  C3/C6 SHA, C6 SPI2/AES, S3 RMT and C6 PARLIO):
  * The owner bit is written back only when OUT_AUTO_WRBACK (OUT_CONF0 bit 2) is set.
    Before, S3 AES/SHA (main) and #203's C3/C6 SHA always cleared it. IDF
    v5.5.5's RMT TX driver sets the bit (`rmt_tx.c:42-46`). Its crypto GDMA
    (`mbedtls/port/crypto_shared_gdma/esp_crypto_shared_gdma.c`) applies no GDMA
    strategy, and PARLIO clears it (`esp_driver_parlio/src/parlio_tx.c:169-173`), so on
    hardware those OUT descriptors keep the owner bit. IDF's AES waits on the IN
    (output) descriptors, which are still written back.
  * A descriptor address that is not word-aligned, or LENGTH > SIZE, is a descriptor
    error (OUT_DSCR_ERR). IDF builds neither.
* **C6 SPI2 and AES** now use the shared gather, so they honour CHECK_OWNER and the
  write-back gate, and the channel stops with OUT_TOTAL_EOF after the transfer. A
  malformed OUT chain still completes the SPI transfer or the AES operation with the
  bytes gathered before the fault, as main does, and GDMA now also raises
  OUT_DSCR_ERR. (The previous head returned without completing; that is reverted.
  Completing keeps main's contract, and no hardware run has shown what SPI2 or AES do
  after an OUT descriptor error.)
* **RMT frame delivery** reaches a pin only through a valid GPIO matrix route:
  IO_MUX MCU_SEL selects function 1 (GPIO matrix), FUNC_OUT_SEL selects the RMT signal,
  and output is enabled (GPIO ENABLE, unless OEN_SEL takes the peripheral's enable).
  Before, the frame went to the first pin whose FUNC_OUT_SEL matched, whatever its IO_MUX
  function or output enable, or to pin 255 when none did.

## Idle path

The retained release AArch64 disassembly on Rust 1.99.0 compares the cold-layout
source on `led-rmt-parlio-fixes` against historical main `ff64902a` and the
#203 tree at `15655505`, now merged as main `a175e357` with an identical tree.
The portable-test follow-up changes no production source, so the parent comparison
applies to the current base. Builds use `cargo +1.99.0 build --release
--bins` and `llvm-objdump -d --demangle --no-show-raw-insn`; `asmcmp.py` normalizes
instruction/branch addresses, alignment nops, adrp page offsets and equivalent
call-target aliases (comparator preserved as [asmcmp.py](asmcmp.py)). [asm.json](asm.json) records all selected symbols and links
complete normalized instruction diffs (large diffs are split into ordered parts).
Whole-binary difference inventories are [main](asm-main-all.txt) and
[parent](asm-parent-all.txt), with base-only/head-only symbols in
[main symbols](asm-main-symbols.json) and [parent symbols](asm-parent-symbols.json).
Each baseline has 30 differing common functions overall; the selected hot-function
set has 10 versus main and 9 versus the parent. This is a code-generation check,
not a timing measurement of the rebased head.

`size_of!` and `offset_of!` dumps in [layout.json](layout.json) cover all named
fields of each chip's `Peripherals` and `SocBus`. Reproduce each revision with
`CARGO_TARGET_DIR=<build-dir> python3 layout_probe.py <checkout>` using the
[preserved probe](layout_probe.py), with no concurrent builds in that checkout.
Every size and field offset
matches main and the parent (185 size/offset records; 258 selected-function
comparisons across both baselines and all three binaries). The only field-name
difference against main is C6's
`i2s0` → `optional`, both at offset 8536. Sizes in bytes:

| Chip | Peripherals | SocBus |
| --- | ---: | ---: |
| C3 | 5136 | 6128 |
| C6 | 8624 | 10176 |
| S3 | 9360 | 12408 |

C6 PARLIO shares the existing boxed `OptionalPeripherals` allocation with I2S and
ECC; I2S remains at offset zero. The nullable PARLIO TX box remains unallocated
until first relevant MMIO access or selected DMA transfer. PCR clock writes before
PARLIO is used allocate nothing, and `clock()` stays `None`, so PARLIO never enters
the optional-device active list. Portable regression tests retain the I2S
offset-zero contract and assert that the actual `Peripherals.optional` field
occupies one pointer, not inline cold-device storage. The nullable Box also
occupies one pointer. No architecture/OS gate or absolute hot-bus size/offset
is pinned, so unrelated field additions do not invalidate the test.

All emitted C3/C6 `Machine::run`, `run_unmodeled`, `step_core`, S3 `run_modeled`,
`run_unmodeled` and `step_core` bodies are instruction-identical to both baselines.
The emitted C3/C6 deadlines and pending-work functions, all peripheral-read bus
wrappers, C3/S3 bus ticks and C3 peripheral read dispatch are also identical.
Inlined accessors (including `devices`, `Peripherals::write32`, S3's deadline and
S3's run dispatch) have no independent emitted symbol: their callers are checked,
not counted as separately identical functions. Both native read/write wrappers
and the S3 access wrappers are included in the comparison.

Every differing selected function is listed below. Counts are normalized
instructions, not executed instructions or a performance estimate.

| Function | main | #203 parent | head | Difference |
| --- | ---: | ---: | ---: | --- |
| C3 `periph_write` | 1660 | 1676 | 1676 | Parent's SHA DMA write hook; identical to parent. |
| C3 `tick_with_pins` | 521 | 521 | 448 | Completed-frame test and cold route fanout replace first-match delivery; register allocation and block order change. |
| C6 bus `tick` | 2036 | 2036 | 1980 | RMT frame fanout is cold; register allocation/block order change, not structure offsets. |
| C6 `periph_write` | 1660 | 1747 | 1822 | Parent ECC/SHA hooks plus PARLIO dispatch and address-gated staging; optional storage indirection. |
| C6 `tick_optional` | 194 | 227 | 229 | Parent ECC arm plus PARLIO dispatch arm and cold-field offsets; PARLIO never joins the active list. |
| C6 `Peripherals::read32` | 679 | 677 | 683 | ECC/PARLIO read dispatch and cold allocation references. |
| S3 bus `write32` | 307 | 307 | 314 | RMT DMA_ACCESS_EN gate and conditional cold staging call. |
| S3 `periph_write_inner` | 1663 | 1663 | 1663 | RMT dispatch calls `RmtDma::write` instead of `Rmt::write`. |
| S3 `tick_impl` | 3667 | 3743 | 3683 | `RmtDma::tick`, completed-frame fanout test/cold call, associated register allocation and block placement changes. |
| S3 `write32_access::<false>` | 194 | 194 | 201 | Same RMT DMA gate/staging hook as bus write32. |

The S3 queue is a nullable boxed `VecDeque` in the former `i64` RMT clock-ratio
space; the ratio is now `u8` in tail padding, preserving `Rmt` and enclosing layouts.
Queue allocation is lazy; reset, DMA disable and staging failure release it.
Plain C3/C6 RMT tick remains the non-DMA specialization; running channels widen the
clock ratio. DMA staging is never a per-tick pump. No claim is made that changed
bus functions are instruction-identical or that their timing is unchanged on the
new base. The historical measured stack and its uncertainty are below.

## Register sources

Public ESP-IDF v5.5.4 sources, with paths relative to `components/`:

| Meaning | Header and lines |
| --- | --- |
| S3 DMA enable bit 25, TX3 only | `soc/esp32s3/register/soc/rmt_struct.h:117-120`; `hal/esp32s3/include/hal/rmt_ll.h:237-240` |
| S3 AFIFO reset bit 23, DMA failure bit 28 | `soc/esp32s3/register/soc/rmt_reg.h:575-581`, `1650-1656` |
| S3 GDMA trigger 9 | `soc/esp32s3/include/soc/gdma_channel.h:21` |
| S3 GDMA auto-writeback, owner check, output interrupts | `soc/esp32s3/register/soc/gdma_reg.h:585-618`, `649-678` |
| C6 PARLIO TX config, width, readiness and interrupts | `soc/esp32c6/register/soc/parl_io_reg.h:161-355` |
| C6 PARLIO PCR clock source, divider, gate, reset | `soc/esp32c6/register/soc/pcr_reg.h:1061-1098`; `hal/esp32c6/include/hal/parlio_ll.h:413-430` |
| C6 bit packing order | `hal/include/hal/parlio_types.h:28-50` |
| C6 approximate RC_FAST 17.5 MHz | `soc/esp32c6/include/soc/clk_tree_defs.h:47` |
| C6 GDMA trigger 9 | `soc/esp32c6/include/soc/gdma_channel.h:17` |
| C6 GDMA auto-writeback, owner check, output interrupts | `soc/esp32c6/register/soc/gdma_reg.h:1665-1716`, `699-732` |
| C6 PARLIO/PCR base addresses | `soc/esp32c6/register/soc/reg_base.h:34,53` |
| C6 PARLIO DATA0 signal 47 | `soc/esp32c6/include/soc/gpio_sig_map.h:84` |
| RMT signal bases S3=81, C3=51, C6=71 | `soc/esp32s3/include/soc/gpio_sig_map.h:158`; C3:84; C6:132 |
| C6 interrupt source 63 | `soc/esp32c6/include/soc/interrupts.h:83-85` |
| DMA descriptor size, length, EOF, owner and next fields | `hal/include/hal/dma_types.h:23-32` |

IDF v4.4.8 S3 `include/soc/rmt_struct.h:126` calls bits 25–31 reserved;
v5.5.4 explicitly names DMA_ACCESS_EN at bit 25. This port follows the IDF 5.x
DMA path, not an Arduino-ESP32 2.x DMA claim. GDMA writeback, owner-check and
interrupt bits match v4.4.8 `include/soc/gdma_reg.h:594-687`. C6 is IDF 5.x.
The PCR header's reset prose and unpack-order prose contain ambiguous values;
the LL reset sequence and `parlio_types.h` enum resolve the modeled meanings.
No hardware verification is claimed.

## Verification

`checks.json` records the final required command results on Rust 1.99.0. Both
workspace invocations use an empty HOME. The ignored-inclusive invocation sets
only ESP32SIM_ROM_DIR among emulator variables, to the checkout's `web/wasm/fw`;
the plain invocation has no emulator variables. CARGO_HOME and RUSTUP_HOME retain
access to the toolchain. The default toolchain is unchanged.

Inputs and source-test hashes are in `inputs.json`. Demo assets were obtained
with `tools/fetch-demo-assets.sh --no-linux` inside this checkout. No firmware
fixtures are copied from another checkout or required from a developer machine.
Existing goldens are unchanged. This branch does not modify JIT code.

The focused tests assert two independent RMT frame deliveries on each chip;
S3 multi-descriptor symbol output and RMT interrupt state; the IDF ping-pong ring
(four halves, owner handback, no descriptor error) and the wait at a CPU-owned ring
node; C6 clock-off prefill, three routed outputs, both lane colours, descriptor
handback and source-63 IRQ assertion/clear. Additional tests cover widths, bit order,
reset, starvation, malformed descriptors, queue bounds, a PCR clock write that must
not allocate PARLIO and the PCR TX reset. The shared C6 gather regression pins SPI
payloads, AES-128 zero-key/zero-input ciphertext, owner handback and completion
interrupt state with automatic writeback enabled and disabled; another C6 test
pins SPI2/AES completion with OUT_DSCR_ERR after a malformed chain. The shared-gather
unit tests pin the write-back gate, descriptor alignment and LENGTH <= SIZE. These
are register-level workloads, not a claim of running a particular FastLED or NeoPixel
firmware release.

`mutations.json` records 45 exact mutations and the tests that kill them, including
the MMIO staging hooks and channel gates, the S3 DMA_ACCESS_EN write gate and CH3
CONF0 address gate, multi-route fanout, DMA mode, reset, the shared gather's
ownership, length, alignment, write-back and EOF rules, the RMT word-alignment and
MMIO-descriptor rules, the ring wait, RESTART resume and continuation past SUC_EOF,
C6 transaction stop, total EOF, descriptor error and SPI2/AES completion after a
fault, queue bounds, the RMT clock ratio and PARLIO packing/clock/IRQ. Storage/idle
mutations force eager PARLIO and RMT DMA allocation (including on a PCR clock write),
retained RMT queue storage after disable, inline PARLIO TX storage and an active
PARLIO clock. Each changed source is restored before the next mutation. Every
mutation fails an assertion in a previously passing focused test. Separately,
[layout-mutation.json](layout-mutation.json) records the cold-allocation guard:
inlining `OptionalPeripherals` fails `cold_devices_do_not_grow_the_native_hot_bus`
on its actual-field size assertion, rather than a compilation failure.
`python3 docs/evidence/led-rmt-parlio-fixes/layout_mutate.py` runs baseline,
mutation and restored-source tests; the restored source passes both tests.

## Limits

DMA staging and descriptor completion occur at MMIO boundaries. Finite chains and
IDF's two-node ping-pong ring are supported. Staging is instantaneous: FIFO
backpressure is stood in for by waiting at a CPU-owned ring node, and concurrent
guest buffer modification is not modeled. S3 rejects a chain whose staged symbols
would exceed the queue bound. PARLIO completes transactionally rather
than after elapsed wire time. It models TX data output, not RX, external input
clocks, valid-signal gating, loop transmission or every PCR reset interaction.
Reserved widths and odd byte counts for 16-bit samples do not complete.

WS2812 high widths of 150–1100 ns, a 550 ns bit split and 50 us reset detection are
inferred decoder policy. A transaction boundary also terminates a parallel frame.
These tolerances and completion rules are not silicon timing claims.

## CPU comparison

The maintainer separately closed #203's C6 idle-path check on merged main:
`ff64902a` → `a175e357`, seven alternating load-gated pairs, median user CPU
3.50 → 3.51 s (+0.3%), after slower 3/7, identical instructions and console
SHA-256. This resolves the parent's earlier +1.37% (7/7) flag; it is not a
remeasurement of #205. The historical #205 measurements below retain their
actual measured parent revisions.

Rust 1.99.0; cargo +1.99.0 build --release --bins; separate target directories. Baseline `schematik-engineering/esp32sim@3d9eee3c` built once; each candidate fetched from origin immediately before its build. Sequential child user CPU via getrusage, including startup. One warmup B→M, then seven measured pairs M→B, B→M alternating. Before every attempt wait for 1-minute load <5 (15-second polling); monitor every second and discard/retry the entire pair if peak >7. Exact total/per-core instructions and console SHA-256 across all attempts. S3 hello: 3000 emulated seconds, board none; C3/C6: 30 seconds, board none; Pocket Tank: 30 seconds, waveshare-amoled18-v2. Ranges are min–max; change is ratio of medians. Flags: slower ≥6/7 or non-overlapping ranges. No per-second load series retained.

Measured branch `led-rmt-parlio-fixes` at `schematik-engineering/esp32sim@fb4568d9167bf42379523dbc8dd39a71961f21ed`, against parent #203 at `schematik-engineering/esp32sim@3d9eee3c`. These are the measured fork SHAs, not the rebased head. The rebase is checked by release disassembly, not re-timed. User CPU seconds; M denotes the measured parent and B the measured branch.

| Workload | Parent median (range) | PR median (range) | Change | PR slower in N/7 | Instructions | max load |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| C3 hello | 2.3717 (2.3383–2.3974) | 2.3683 (2.3406–2.3976) | -0.14% | 3/7 | 4800000000 | 4.31 |
| C6 hello | 3.0813 (3.0613–3.1100) | 3.0896 (3.0707–3.1219) | +0.27% | 4/7 | 4800000000 | 4.76 |
| Pocket Tank | 35.5661 (35.2486–36.3006) | 35.3132 (34.9827–35.8873) | -0.71% | 1/7 | 10073833665 | 5.20 |
| S3 hello | 24.3777 (24.2110–25.2191) | 24.1693 (23.9120–25.5136) | -0.85% | 2/7 | 1789819657 | 5.12 |

### C3 hello

Status: PASS. Flags: —.

| Pair | Attempt | Order | Parent s | PR s | Parent load before/after/peak | PR load before/after/peak | Result |
| --- | ---: | --- | ---: | ---: | --- | --- | --- |
| warmup | 1 | B→M | 2.384789 | 2.431075 | 4.31/4.21/4.31 | 4.31/4.31/4.31 | accepted |
| 1 | 2 | M→B | 2.393159 | 2.359975 | 4.21/4.21/4.21 | 4.21/4.03/4.21 | accepted |
| 2 | 3 | B→M | 2.371702 | 2.368266 | 4.03/3.95/4.03 | 4.03/4.03/4.03 | accepted |
| 3 | 4 | M→B | 2.397354 | 2.363437 | 3.95/3.95/3.95 | 3.95/3.95/3.95 | accepted |
| 4 | 5 | B→M | 2.352910 | 2.374586 | 3.95/4.04/4.04 | 3.95/3.95/3.95 | accepted |
| 5 | 6 | M→B | 2.338348 | 2.340565 | 4.04/4.04/4.04 | 4.04/4.04/4.04 | accepted |
| 6 | 7 | B→M | 2.384855 | 2.371508 | 4.11/4.11/4.11 | 4.04/4.11/4.11 | accepted |
| 7 | 8 | M→B | 2.348362 | 2.397557 | 4.11/4.18/4.18 | 4.18/4.18/4.18 | accepted |

### C6 hello

Status: PASS. Flags: —.

| Pair | Attempt | Order | Parent s | PR s | Parent load before/after/peak | PR load before/after/peak | Result |
| --- | ---: | --- | ---: | ---: | --- | --- | --- |
| warmup | 1 | B→M | 3.105181 | 3.070354 | 4.25/4.39/4.39 | 4.18/4.25/4.25 | accepted |
| 1 | 2 | M→B | 3.105535 | 3.121899 | 4.39/4.39/4.39 | 4.39/4.44/4.44 | accepted |
| 2 | 3 | B→M | 3.110040 | 3.102028 | 4.48/4.48/4.48 | 4.44/4.48/4.48 | accepted |
| 3 | 4 | M→B | 3.078559 | 3.089638 | 4.48/4.44/4.48 | 4.44/4.44/4.44 | accepted |
| 4 | 5 | B→M | 3.061252 | 3.084151 | 4.57/4.76/4.76 | 4.44/4.57/4.57 | accepted |
| 5 | 6 | M→B | 3.092194 | 3.088289 | 4.76/4.76/4.76 | 4.76/4.70/4.76 | accepted |
| 6 | 7 | B→M | 3.079297 | 3.116731 | 4.72/4.72/4.72 | 4.70/4.72/4.72 | accepted |
| 7 | 8 | M→B | 3.081324 | 3.070723 | 4.72/4.59/4.72 | 4.59/4.59/4.59 | accepted |

### Pocket Tank

Status: PASS. Flags: —.

| Pair | Attempt | Order | Parent s | PR s | Parent load before/after/peak | PR load before/after/peak | Result |
| --- | ---: | --- | ---: | ---: | --- | --- | --- |
| warmup | 1 | B→M | 36.023487 | 35.400990 | 4.56/4.55/4.56 | 4.59/4.56/4.64 | accepted |
| 1 | 2 | M→B | 36.300569 | 35.313242 | 4.55/4.36/4.83 | 4.36/4.44/4.49 | accepted |
| 2 | 3 | B→M | 35.568462 | 35.318085 | 4.12/3.92/4.12 | 4.44/4.12/4.48 | accepted |
| 3 | 4 | M→B | 35.287419 | 34.982686 | 3.92/3.97/4.46 | 3.97/4.38/4.38 | accepted |
| 4 | 5 | B→M | 35.566091 | 35.500187 | 4.42/5.20/5.20 | 4.38/4.42/4.43 | accepted |
| 5 | 6 | M→B | 35.345058 | 35.198881 | 4.99/4.58/4.99 | 4.58/4.98/4.98 | accepted |
| 6 | 7 | B→M | 35.248649 | 35.239203 | 4.88/4.73/4.93 | 4.98/4.88/5.03 | accepted |
| 7 | 8 | M→B | 35.679336 | 35.887337 | 4.73/4.67/4.82 | 4.67/4.21/4.70 | accepted |

### S3 hello

Status: PASS. Flags: —.

| Pair | Attempt | Order | Parent s | PR s | Parent load before/after/peak | PR load before/after/peak | Result |
| --- | ---: | --- | ---: | ---: | --- | --- | --- |
| warmup | 1 | B→M | 24.246281 | 24.163412 | 4.08/4.15/4.15 | 4.21/4.08/4.21 | accepted |
| 1 | 2 | M→B | 24.377719 | 23.958384 | 4.15/4.66/4.66 | 4.66/4.81/4.81 | accepted |
| 2 | 3 | B→M | 24.384050 | 23.974155 | 4.52/4.85/4.85 | 4.81/4.52/4.81 | accepted |
| 3 | 4 | M→B | 24.210977 | 23.911955 | 4.85/4.42/4.85 | 4.42/3.91/4.42 | accepted |
| 4 | 5 | B→M | 24.307576 | 24.424526 | 4.15/4.72/4.72 | 3.91/4.15/4.25 | accepted |
| 5 | 6 | M→B | 24.442854 | 24.357124 | 4.72/4.49/4.72 | 4.49/4.74/4.80 | accepted |
| 6 | 7 | B→M | 24.261986 | 24.169314 | 5.01/4.94/5.01 | 4.74/5.01/5.10 | accepted |
| 7 | 8 | M→B | 25.219083 | 25.513578 | 4.94/5.02/5.12 | 5.02/4.81/5.10 | accepted |

Max load in summary includes accepted warmup and measured pairs; discarded attempts appear above.


## Evidence privacy

Retained evidence contains public header paths, hashes, commands and outcomes.
Local build logs and filesystem identities are omitted. No raw private captures
or hardware identifiers are retained.
