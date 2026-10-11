# EX219: LED routing and DMA output

Base: #203's branch `c3-c6-sha-dma-wifi-cal` at `schematik-engineering/esp32sim@3d9eee3c`,
stacked on #201's `i2s-rx-input` (`baf9845b`) on upstream main `eec7b2c9`. Related EX205 provides CPU-fed
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

Checked by disassembly, not timing: release binaries of main `2f9443a9` and this
branch (`cargo +1.99.0 build --release -p esp32sim --bins`, fat LTO, one codegen
unit), `objdump -d --demangle`, per-function comparison with instruction and
branch-target addresses and adrp page offsets normalised. 1856 functions are
identical; the S3 differences are listed here.

`Rmt` and every structure embedding it keep main's size and field offsets. The
TX3 queue is a nullable boxed `VecDeque` inside `Rmt`; it takes the 8 bytes of the
former `i64` CPU/APB ratio, which is now a `u8` in the tail beside `running`
(`running` stays at the same offset). S3 `Peripherals`, `SocBus`, the CPU run
loop, JIT helpers and bus read/fetch paths therefore compile to main's
instructions. The queue is allocated when staged symbols are inserted; FIFO
reset, DMA disable and a staging failure release it. S3 uses `RmtDma`, a
`repr(transparent)` wrapper, to select the DMA transmitter variant and to clear
the queue on CH3 CONF0 writes.

The transmitter loop is one inlined, const-specialized function. Plain
`Rmt::tick` (C3/C6) instantiates only the non-DMA variant. `RmtDma::tick` is out
of line like main's `Rmt::tick`; its idle path (prologue, `running` load, branch)
is instruction-identical, and `tick_impl` calls it where main calls `Rmt::tick`.
The only other `tick_impl` change on the idle path is the completed-frame test:
the route fanout is a cold out-of-line function, so the test branches over a call
instead of over main's inlined loop.

`periph_write_inner` is identical to main except the RMT dispatch target
(`RmtDma`'s write). Staging sits in the `periph_write` wrapper behind one bit
test of CH3 CONF0 DMA_ACCESS_EN: three instructions (offset, byte load, branch)
per peripheral write while RMT DMA is off. With the bit set, a cold function
checks the address: CH3 CONF0, or a GDMA write to a running trigger-9 channel.
Writes to other GDMA channels (SPI2, LCD, I2S, camera, crypto) do not stage.
No DMA pump or staging call runs from a device tick.

Remaining S3 differences: `gather_dma_out` carries the descriptor alignment,
length and trigger-9 checks plus the writeback gate (AES, SHA and RMT
transactions only; SPI2, LCD, I2S and camera use other walkers); `Rmt::tick`
widens the `u8` ratio on running channels; construction and drop code.
C3's `tick_with_pins` tests the completed-frame queue before calling a cold
fanout function.

C6 adds a pointer-sized PARLIO handle to `Peripherals`, which shifts later fields
by 8 bytes; the C6 bus and run-loop functions differ by those offsets. The PARLIO
device adds read/write dispatch arms. The C6 write path classifies PARLIO, GDMA
and the PCR PARLIO clock address inline and stages in a cold function. C6
`Peripherals::write32` gets no PARLIO code: the PCR PARL_CLK_TX_CONF value is read
and applied only when staging.

PARLIO keeps a nullable boxed TX state at the end of `Peripherals`. Before its
first MMIO access or selected DMA transfer, no TX state or register RAM is
allocated. The pointer-sized handle avoids placing the 128-byte TX state among
hot fields through Rust's field reordering. Layout tests require the handle to
follow the system timer, clock tree and cached interrupt status.

PARLIO `clock()` remains `None`, including during a configured transfer, so
PARLIO never enters the optional-device active list. A PCR clock write before
PARLIO is used allocates no TX state.

Source 63 uses the existing optional interrupt cache in status word 1, which
already contains LEDC and MCPWM. The `OPTIONAL_SOURCES` mask admits PARLIO's
cached bit without adding a device callback or another status-word load. Shared
interrupt routing is identical to upstream. Tests cover PARLIO's routed assertion
and acknowledgement, and prove the unused device has no storage, clock or active
entry.

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
mutation fails an assertion in a previously passing focused test.

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

Rust 1.99.0; cargo +1.99.0 build --release --bins; separate target directories. Main 2f9443a9 built once; each candidate fetched from origin immediately before its build. Sequential child user CPU via getrusage, including startup. One warmup B→M, then seven measured pairs M→B, B→M alternating. Before every attempt wait for 1-minute load <5 (15-second polling); monitor every second and discard/retry the entire pair if peak >7. Exact total/per-core instructions and console SHA-256 across all attempts. S3 hello: 3000 emulated seconds, board none; C3/C6: 30 seconds, board none; Pocket Tank: 30 seconds, waveshare-amoled18-v2. Ranges are min–max; change is ratio of medians. Flags: slower ≥6/7 or non-overlapping ranges. No per-second load series retained.

Measured on `schematik-engineering/esp32sim@46b0d94c` (branch `led-rmt-parlio-fixes` before its rebases onto #203) against main `2f9443a9`. An earlier run of the same head had run-to-run drift of up to 37% between pairs on Pocket Tank and was repeated; only the repeat is reported. The current head was not re-timed; the idle-path section above gives its disassembly comparison. C3 hello, whose `tick_with_pins` changed, is timed centrally: <C3_TABLE>. User CPU seconds.

| Workload | Main median (range) | PR median (range) | Change | PR slower in N/7 | Instructions | max load |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Pocket Tank | 36.0615 (34.7514–39.3186) | 35.1536 (34.6300–38.5041) | -2.52% | 3/7 | 10073833665 | 4.30 |
| C6 hello | 3.1824 (3.0831–3.5194) | 3.1577 (3.0538–3.4191) | -0.77% | 3/7 | 4800000000 | 3.85 |
| S3 hello | 25.0508 (24.1710–29.4907) | 24.7047 (24.1888–29.0515) | -1.38% | 4/7 | 1789819657 | 6.38 |

### Pocket Tank

Status: PASS. Flags: —.

| Pair | Attempt | Order | Main s | PR s | Main load before/after/peak | PR load before/after/peak | Result |
| --- | ---: | --- | ---: | ---: | --- | --- | --- |
| warmup | 1 | B→M | 34.346091 | 34.444260 | 3.53/2.67/3.53 | 4.30/3.53/4.30 | accepted |
| 1 | 2 | M→B | 34.754824 | 35.084599 | 2.67/1.93/2.67 | 1.93/2.07/2.10 | accepted |
| 2 | 3 | B→M | 35.937203 | 34.629986 | 1.95/2.16/2.19 | 2.07/1.95/2.21 | accepted |
| 3 | 4 | M→B | 34.751428 | 35.067585 | 2.16/1.97/2.16 | 1.97/2.22/2.22 | accepted |
| 4 | 5 | B→M | 39.318593 | 35.454750 | 2.27/3.67/3.88 | 2.22/2.27/2.27 | accepted |
| 5 | 6 | M→B | 36.297424 | 35.153575 | 3.67/3.75/4.17 | 3.75/3.54/3.92 | accepted |
| 6 | 7 | B→M | 36.061525 | 35.233183 | 3.35/2.91/3.35 | 3.54/3.35/3.68 | accepted |
| 7 | 8 | M→B | 36.490135 | 38.504077 | 2.91/2.98/3.00 | 2.98/3.15/3.28 | accepted |

### C6 hello

Status: PASS. Flags: —.

| Pair | Attempt | Order | Main s | PR s | Main load before/after/peak | PR load before/after/peak | Result |
| --- | ---: | --- | ---: | ---: | --- | --- | --- |
| warmup | 1 | B→M | 3.113898 | 3.111458 | 3.15/3.05/3.15 | 3.15/3.15/3.15 | accepted |
| 1 | 2 | M→B | 3.083412 | 3.133376 | 3.05/2.97/3.05 | 2.97/2.97/2.97 | accepted |
| 2 | 3 | B→M | 3.199634 | 3.418288 | 3.29/3.11/3.29 | 2.97/3.29/3.29 | accepted |
| 3 | 4 | M→B | 3.182382 | 3.278481 | 3.11/3.11/3.11 | 3.11/3.66/3.66 | accepted |
| 4 | 5 | B→M | 3.519430 | 3.419116 | 3.85/3.85/3.85 | 3.66/3.85/3.85 | accepted |
| 5 | 6 | M→B | 3.305069 | 3.157726 | 3.85/3.78/3.85 | 3.78/3.64/3.78 | accepted |
| 6 | 7 | B→M | 3.087312 | 3.076582 | 3.64/3.51/3.64 | 3.64/3.64/3.64 | accepted |
| 7 | 8 | M→B | 3.083069 | 3.053815 | 3.51/3.46/3.51 | 3.46/3.46/3.46 | accepted |

### S3 hello

Status: PASS. Flags: —.

| Pair | Attempt | Order | Main s | PR s | Main load before/after/peak | PR load before/after/peak | Result |
| --- | ---: | --- | ---: | ---: | --- | --- | --- |
| warmup | 1 | B→M | 24.542803 | 24.370655 | 3.76/3.64/3.76 | 3.46/3.76/3.90 | accepted |
| 1 | 2 | M→B | 25.599767 | 26.881910 | 3.64/3.63/3.75 | 3.63/3.89/4.15 | accepted |
| 2 | 3 | B→M | 25.050774 | 24.704659 | 3.79/3.85/4.01 | 3.89/3.79/3.89 | accepted |
| 3 | 4 | M→B | 25.417101 | 29.051514 | 3.85/4.53/5.08 | 4.53/6.38/6.38 | accepted |
| 4 | 5 | B→M | 29.490685 | 26.877363 | 4.09/5.35/5.56 | 4.91/4.09/4.91 | accepted |
| 5 | 6 | M→B | 33.510618 | 42.946415 | 4.34/6.27/6.68 | 6.27/13.35/13.35 | discarded: load >7 |
| 5 | 7 | M→B | 34.516602 | 50.924342 | 4.77/5.57/5.57 | 5.57/11.45/11.89 | discarded: load >7 |
| 5 | 8 | M→B | 26.107778 | 25.741450 | 4.34/7.55/7.55 | 7.55/5.65/7.55 | discarded: load >7 |
| 5 | 9 | M→B | 24.726422 | 29.589800 | 4.62/5.57/6.21 | 5.57/6.86/7.19 | discarded: load >7 |
| 5 | 10 | M→B | 24.747253 | 24.349590 | 4.66/4.97/4.97 | 4.97/3.62/4.97 | accepted |
| 6 | 11 | B→M | 24.170993 | 24.188773 | 2.72/2.37/2.72 | 3.62/2.72/3.62 | accepted |
| 7 | 12 | M→B | 24.312944 | 24.684476 | 2.37/3.46/3.46 | 3.46/3.07/3.46 | accepted |

Max load in summary includes accepted warmup and measured pairs; discarded attempts appear above.



## Evidence privacy

Retained evidence contains public header paths, hashes, commands and outcomes.
Local build logs and filesystem identities are omitted. No raw private captures
or hardware identifiers are retained.
