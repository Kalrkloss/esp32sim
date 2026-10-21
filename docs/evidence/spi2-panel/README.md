# A panel on GP-SPI2: what a real display driver's transfers look like

**Question.** An open question came out of the access-point work (EX232): a guest configured through
its own web UI to drive an SPI panel reports the configured module on its next boot, its driver
brings SPI2 up — and then `--tft-png` is a frame of black, `[emu] tft: 0 RAMWR, 0 pixels`. Nothing
of the panel ever reached the model. Is the model missing the transfer path — does the guest block
on a status bit the model never sets, or write a register the model ignores?

**Answer.** Neither, and the model already carries the path. On this host the panel has two feeds and
both are modelled: the bit-banged pins the board was brought up against, and the GP-SPI2 peripheral
(`BoardModel::spi_tx(2, …)` in `esp32s3/src/board/atech14.rs`, documented in `docs/decisions.md`
under *Boards and firmware*). A released image whose interpreter has the display commands configures
the panel through its own web UI, initialises it over SPI2 and draws: the run below decodes **4 RAMWR,
73 776 pixels**, and `--tft-png` is the filled rectangle recorded in [`frame.txt`](frame.txt).

The image the earlier run used does bring SPI2 up — that part of the observation was right — and then
writes no `SPI_CMD` at all: 55 register accesses for the whole run, none of them a transaction. What
it lacks is the display vocabulary: its interpreter rejects `tft.init`, `tft.fill`, `tft.print`,
`tft.setfreq` and `gui.init` as syntax errors while accepting `print`, `pin.mode` and `spi.setup`, so
nothing ever asks the bus for a transfer. The boot line that looked like progress
(`module … Display 4`) is a *configuration* report.

Two things turned that into "the model is missing the transfer path", and the first is fixed here:

1. **`--regstat` went blind at the guest's first reset.** A guest reset rebuilds the digital
   peripheral set, and the log an MMIO observer reads lives in it; only the debug areas were carried
   over. Every access after the reset — i.e. the whole second boot, the one that has the panel
   configured — was dropped. Without this receipt's first fix, the 55 accesses below do not exist.
2. **The symptom was read one layer too high.** "The driver configures SPI2 and starts no transfer"
   is true of that image and says nothing about the model: a driver needs a command to send. Repeating
   the same flow with an image that has the commands turns the same registers into 16 195
   transactions and a captured frame.

## Code

- `soc: keep an MMIO observer's register log across a guest reset` (`esp32s3`, `esp32c6`, `esp32c3`,
  `esp32` `src/soc.rs`): `reboot` carries `misc.mmio_log` (history included) like it already carried
  `misc.log_unknown` and the debug areas. Host-side state only — no guest-visible behaviour, no
  instruction cost.
- `esp32s3/tests/reboot.rs` — the reset keeps the log armed and records the first access after it.
  Removing the carry from `esp32s3/src/soc.rs` makes the test fail (`mmio_log` is `None` after
  `reboot`), so it pins the mechanism rather than the symptom.
- `esp32s3/src/bus/tests.rs` — `spi2_cpu_transfer_reaches_the_panel_board`: `CMD` with `USR` set and
  `UPDATE` alongside it, the way `spi_master` writes both after filling `USER`, the W registers and
  `MS_DLEN`, with the board's D/C line: one transaction per command, two pixels in RAM, both in the
  visible frame, `TRANS_DONE` pending.

No change to the board, the SPI peripheral model or the panel decoder: they already carry the path,
and the register table below is what a driver does to them. 984 workspace tests and both Clippy
checks (native and wasm) pass with the two commits; the goldens — console text, audio hashes and
instruction counts of the demo firmware — are unchanged, which is the "code not in use costs
nothing" bar for a change that only touches the reset path's host state.

## The register diff: what the driver writes, and what the model does with it

`--regstat` over the whole run (12 emulated seconds, both boots), SPI2 at `0x6002_4000`
(`esp32s3/src/periph.rs` block `0x24` → `esp_periph::GpSpi`); full table in
[`spi2-registers.txt`](spi2-registers.txt):

| offset | register | writes | reads | values written | model |
| --- | --- | --- | --- | --- | --- |
| `0x000` | `SPI_CMD` | 16 218 | 16 533 | `0x00800000` (`UPDATE`), `0x01800000` (`UPDATE\|USR`) | `write(0)`: `USR` (bit 24) starts one transaction; `UPDATE` (23) is folded in, `USR` reads back 0 once the board has taken it — the handshake the driver polls |
| `0x00c` | `SPI_CLOCK` | 29 | 2 | `0x00003043`, `0x003c4002` | register RAM; `wire_source_cycles` derives the divider from it |
| `0x010` | `SPI_USER` | 64 | 11 390 | `0x08000000`, `0x18000000`, `0x18000001` | read-modify-write of the phase enables; `spi_transfer` decodes `USR_MOSI`/`USR_MISO`/`DOUTDIN` from it |
| `0x014` | `SPI_USER1` | 7 | 2 | `0x00000000` | register RAM (address-phase length, unused here) |
| `0x01c` | `SPI_MS_DLEN` | 12 568 | 0 | `0x7`…`0x1ff` (8…512 bits) | the length of the data phase, as `spi_ll_set_mosi_bitlen` writes it on this host |
| `0x020` | `SPI_MISC` | 29 | 3 | `0x3f` | register RAM (CS idle/keep-active); MISO length is not a separate register here |
| `0x030` | `SPI_DMA_CONF` | 28 | 22 | `0x0`, `0x60000000`, `0x60180000`, `0x00100000` | `DMA_TX_ENA`/`DMA_RX_ENA`: with TX enabled the bus fetches the data phase through GDMA, otherwise from W |
| `0x034` | `SPI_DMA_INT_ENA` | 1 | 1 | `0x00001000` | `int_ena`: bit 12 is `TRANS_DONE`, the only source this driver enables |
| `0x044` | `SPI_DMA_INT_SET` | 1 | 1 | `0x00001000` | peripheral RAM: the write-1-to-set alias of the same word, written and read back once. Nothing waits on it — the driver's completion is `CMD` reading idle with `TRANS_DONE` set |
| `0x098..0x0d4` | `SPI_W0..W15` | 28 243 | 0 | command bytes (`0x2c`), pixel words (`0x00f800f8`), gamma tables | the data phase, in order, from `W0` |
| `0x0e8` | `SPI_CLK_GATE` | 17 | 12 | `0x0`, `0x1`, `0x5`, `0x7` | register RAM |

Nothing in that list is unimplemented in a way that changes the outcome: 16 195 transactions on the
wire, each started by one `SPI_CMD` write, each retired (`CMD` reads 0, `TRANS_DONE` set) and the
byte stream decoded by `DcsPanel`. The init sequence arrives one byte per transaction, D/C low for
commands, and the decode is the controller's own:

```
[spi2] transfer tx=1 rx=0: [3a]      COLMOD
[spi2] transfer tx=1 rx=0: [55]      16-bit pixels
[spi2] transfer tx=1 rx=0: [36]      MADCTL
[spi2] transfer tx=1 rx=0: [6c]      MV|MX|MH|BGR
[spi2] transfer tx=1 rx=0: [11]      SLPOUT
[spi2] transfer tx=1 rx=0: [b1] [01] [2c] [2d]   gamma / power, then c2,c3,c4,ee …
```

## The frame

Guest handled end to end through its own page (EX232's join and forward), 12 emulated seconds:

```
--- tft.init 1
--- tft.fill 0
--- tft.rect 0, 0, 160, 80, 0xF800, 1
[emu] tft: 4 RAMWR, 73776 pixels, madctl=0x6c inverted=false on=true bbox=Some((50, 3, 129, 161))
           top colours [(f800, 31b0)]; gpio events 11614
[emu] wrote …/frame.png
```

[`frame.txt`](frame.txt) is the capture (480×240, the `--tft-png` 3× scale of the board's 160×80
glass; `*.png` is not committed here, so the file carries the numbers, the colour histogram and an
80×40 map of where the pixels are): 79 632 red pixels, 35 568 black, one filled rectangle across the
upper two thirds of the panel and black elsewhere — rows 0…26 of 40 are filled, start to finish.
`bbox` is the panel's own RAM, so the rectangle's edges are the driver's window arithmetic, not the
capture's crop.

## Reaching the run

```
esp32sim --chip s3 --boot rom --flash-mb 16 --psram-mb 8 --board atech14 --console uart0 \
  --rom $ROM --bootloader $FW_BOOTLOADER --ptable $FW_PTABLE --app $FW_APP \
  --flash-at 0xe000=$FW_BOOT_APP0 --max-seconds 12 --tft-png frame.png --regstat regstat.txt \
  --wifi-join '' --hostfwd tcp:127.0.0.1:PORT-192.168.4.1:80 --debug spi
```

then, from the host: `GET /config`, `POST /_kfg_` with
`MODULE=1&DISPLAY=4&ORIENTATION=1&SDCARD=0&PINS=1,-1,2,41,40,-1,-1,-1,-1,-1,-1,-1` (the board's
MOSI/SCLK/CS/DC), `GET /config?restart=yes`, wait for the page to return, and drive the guest's own
WebSocket console (`/ws`, subprotocol `Output`, `cmd:immediate <line>`) with the three drawing lines
above. [`probe-panel.py`](probe-panel.py) is exactly that driver, with the firmware paths taken from
the environment.

Images (unmodified, released; 16 MB flash, 8 MB PSRAM, S3 mask ROM `esp32s3_rev0_rom.elf`
`c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd`):

| | app | bootloader | partition table | boot_app0 |
| --- | --- | --- | --- | --- |
| A (older) | `645a7e932be51aac212081890ece17ef56fceee829c2e060883525b942389109` | `7c2892c8…` | `24db2c95…` | `f94c5d78…` |
| B (newer) | `128d0209a236ecd3ca30a51b4eb9e3e00503bbf3a7a1c71bb472c017c88d6bc6` | `7c2892c8…` | `24db2c95…` | `f94c5d78…` |

Image A is the one the access-point receipt used. Its boot after the form POST reports the
configured module and brings SPI2 up, and its interpreter answers the display lines like this
(markers are `print` lines sent around them):

```
--- print 100            ->  100
--- tft.init 1           ->  ERR:Syntax error
--- print 101            ->  101
--- tft.fill 0           ->  ERR:Syntax error
--- tft.print "X"        ->  ERR:Syntax error
--- print 103            ->  103
[emu] tft: 0 RAMWR, 0 pixels, madctl=0x0 inverted=false on=false bbox=None …
```

Its whole SPI2 traffic, from the fixed probe — the bus and device setup, no `SPI_CMD`, no
transaction, nothing polled:

| offset | register | accesses | values |
| --- | --- | --- | --- |
| `0x008` | `SPI_CTRL` | 5 | `0x0` |
| `0x00c` | `SPI_CLOCK` | 2 | `0x003c4002` (80 MHz) |
| `0x010` | `SPI_USER` | 9 | `0x18000001` |
| `0x014` | `SPI_USER1` | 1 | `0x0` |
| `0x020` | `SPI_MISC` | 3 | `0x0` |
| `0x030` | `SPI_DMA_CONF` | 11 | up to `0x60180000` |
| `0x098..0x0d4` | `SPI_W0..W15` | 16 | all zero: the W registers cleared |
| `0x0e0` | `SPI_SLAVE` | 1 | `0x0` |
| `0x0e8` | `SPI_CLK_GATE` | 7 | `0x0`, `0x1`, `0x5`, `0x7` |

That is the sequence the access-point receipt quotes (`DMA_CONF 0x60180000`, `USER 0x18000001`,
`W0..W15` cleared, `CLOCK 0x003c4002`) — read there through `--debug mmio`, which survives a reset,
and read here through a `--regstat` that no longer loses the second boot.

So the guest never blocks and never depends on a status bit the model does not set: it initialises
the bus and stops, because in that build there is no display code above it to send anything.

Speed: image B runs at 12.5 Minsn/s here (12.0 emulated seconds in 198 s wall, ≈0.06× real time), so
every run is budgeted in emulated seconds.

## Limits

- **Not measured against hardware.** The claim is that a released driver's transfers reach the
  modelled panel and are decoded; whether the panel model's colours and window offsets match a real
  module is the board model's own question (`docs/decisions.md`), not this one.
- **Colour interpretation is the controller's.** The capture above is a 16-bit `0xF800` fill; the
  model's `top colours` are RAM values, and `tft_frame_160x80` maps the module's visible window. A
  driver using another geometry (the same controller in 128×160 form fills RAM columns 2..129) still
  produces a frame, but its pixels lie where that geometry puts them.
- **`DMA_INT_SET` (`0x044`) stays peripheral RAM.** This driver writes it once and polls `CMD` and
  `INT_ST`; a driver that waits on a set-only bit it wrote there would not see it. Nothing in these
  runs does.
- **The board feeds the panel from every byte on the bus**, not gated by the software chip-select on
  GPIO 41 (`spi_tx` takes the whole transaction); a driver that interleaves a second device on the
  same host would need the pin-routed path the C6 and classic boards use (`SpiPins`/`spi_transfer_pins`).
- **One image, one driver path.** Image B is one released build; another toolchain's `spi_master`
  call sequence is the same API and the same registers, but it was not run here.
- **`tft.print` over the guest's immediate console draws and then panics** in image B
  (`LoadProhibited`, `EXCVADDR` is the ASCII of the command name); `fill` and `rect` do not. Not
  investigated — the drawing lands before it — but a re-run that uses `print` will end with a core
  dump rather than a clean exit line.
- **The carry fixes the probe, not the guest.** If a future run still shows no SPI2 access, the
  probe can now be trusted; the previous silence was ours.
