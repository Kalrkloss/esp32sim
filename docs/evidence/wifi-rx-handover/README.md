# Receive lists handed over one at a time (S3 MAC)

**Question.** HeishaMon v4.2.2 (Arduino-ESP32 3.x, released `esp32-v4.2.2` image) boots and runs in
esp32sim but its station never associates: the virtual AP answers probe requests, the station never
authenticates, and the end-of-run report reads `... 1 received (314 dropped: no descriptor); AP: ...
0 data frames from the station, state Idle`. The ESP-IDF station specimen on the same blob associates
normally. Why does the receive path stop after the first frame?

**Answer.** Arduino-ESP32 hands the receive DMA **one short descriptor list at a time** instead of one
long ring, and starts the next list by writing `BASE_RX_DSCR` (MAC `0x60033088`). The model treated a
base write as a no-op unless `rx_next` was still null, so after the first list was consumed the DMA
stayed pointed at an unarmed descriptor for ever and every later frame was dropped. A stopped DMA now
takes the pending base as its new head and retries the frame
(`esp32s3/src/bus.rs::wifi_rx_deliver`, `esp32s3/src/periph.rs` `WifiMac::write`).

## What the two drivers do differently

Both firmwares drive the same modelled MAC (0x60033000 block) through Espressif's closed
`libpp`/`libnet80211` blob, but from different IDF generations: HeishaMon v4.2.2 is built against
`idf-release_v5.1-3662303f31` (`v5.1.2-185-g3662303f31-dirty`, from the image's own strings), the
station specimen against **IDF 5.5.4**. The receive-list management differs between them; the virtual
AP, channel and blob API do not.

| | IDF station specimen (`examples/wifi-station`) | HeishaMon v4.2.2 |
| --- | --- | --- |
| `BASE_RX_DSCR` (0x088) writes | 1, at start-up | one per list, changing address |
| descriptor stride / buffer size | 12 bytes, 1600 (`dw0=0x80640640`) | 12 bytes, 2212 (`dw0=0x808a48a4`) |
| list length | long ring, all descriptors armed | 4 descriptors, last one's `next` null |
| restart signal | `0x084` bit 0 after every frame | `0x088` write of the next list's head |
| frames dropped by the model before the fix | 0 | 314 of 315 |

Traced with `--debug wifi` (every MAC register write) and the RX path's drop line:

```
[wifi] wr 0x33+0x088 <- 0x3fcb3f0c            # list 1, one usable descriptor
[wifi] RX -> desc 0x3fcb3f0c 108 bytes beacon (0/8) ...
#   desc 0x3fcb3f0c: dw0=0x808a48a4 (size 2212, owner 1), buf=0x3fcb3f4c, next=0x3fcb3f18
#   desc 0x3fcb3f18: dw0=0, buf=0, next=0      -> the DMA has nothing to fill: stopped
[wifi] wr 0x33+0x088 <- 0x3fcb6570            # list 2, head written *before* the next frame is due
[wifi] RX dropped: rx_next 0xb3f18 is not a usable descriptor (base 0xb6570, last 0x10b3f0c)
#   list 2 at 0x3fcb6570: four armed descriptors (dw0=0x808a48a4, last next=0)
```

The second write arrives while the DMA is still processing the first list, so a model that looks at the
base only when a frame is due has to remember it: hence `WifiMac::rx_handover`, set by a base write,
cleared by the next frame that goes in.

## Numbers

Emulated 25 s, one released HeishaMon image, `--wifi ssid=heishamon-bench,chan=6,psk=bench-password`,
`--max-seconds 25`, CN-CNT answer replayed into UART1 from 10 s to 30 s.

| | before | after |
| --- | --- | --- |
| end-of-run `[emu] wifi:` | 166 sent, **1 received, 314 dropped**, AP 244 beacons / 71 probe responses / 0 data frames, `state Idle` | 226 sent, **570 received, 1 dropped**, AP 244 beacons / 22 probe responses / 169 data frames, `state Associated` |
| four-way handshake | never reached | `[wifi] WPA2 four-way handshake complete` |
| `[emu] net:` | 0 DHCP leases | 1 DHCP lease, 2 ARP replies |
| `[emu] nat:` | 0 TCP connections | 1 TCP connection (MQTT), 3 UDP flows, 8541 bytes out |
| firmware console | `Wifi: -1% (RSSI: 0)`, `MQTT connect failed` | `Wifi: 100% (RSSI: -40)`, `Connecting to mqtt server ...` |
| MQTT topics seen on the host | none | `heishamon/ip 10.0.2.15`, 144 `heishamon/main/...` values in the window |

The IDF station specimen is unchanged by the fix: `46 sent, 146 received (0 dropped), state
Associated`, `sta ip: 10.0.2.15`.

## Commands

```sh
# the specimen that never starved, before and after the change (binaries from the browser fixtures)
esp32sim --board none --chip s3 --boot rom --console usb --max-seconds 12 \
  --rom <roms>/esp32s3_rev0_rom.elf --bootloader s3-wifi-bootloader.bin \
  --ptable s3-wifi-ptable.bin --app s3-wifi_station.bin --wifi ssid=esp32sim,psk=esp32sim-pass

# the firmware that starved: released image, LittleFS partition, debug areas used above
esp32sim --chip s3 --boot rom --console usb --debug wifi,wifi-frames --max-seconds 25 \
  --rom <roms>/esp32s3_rev0_rom.elf --bootloader HeishaMon.ino.bootloader.bin \
  --ptable HeishaMon.ino.partitions.bin --app HeishaMon.ino.esp32-v4.2.2.bin \
  --flash-at 0x3d0000=littlefs.img --flash-mb 4 --psram-mb 2 --board none \
  --wifi ssid=heishamon-bench,chan=6,psk=bench-password --script cn-cnt-replay.txt

# the unit tests that pin both halves of the rule
cargo test -p esp32s3 --lib bus::gp_spi_board_tests::air_rx

# against the code before the change, the first of those two tests fails at
#   "the base write restarts the stopped DMA": left 1, right 2
```

## Inputs and their hashes

The HeishaMon build is the project's own released binary and is **not** redistributed here; the image
was not modified (these hashes are the inputs to every number above):

```
627aa40588f9cf4c58ffe2cb1188ac47ada72b6ce2e9a4a0e2b80862a5f06099  HeishaMon.ino.esp32-v4.2.2.bin (1574704 bytes)
2382948f539c8753a3f56f2e9f03ab182803610c138e08e5d8e7fcc822bf9ba2  HeishaMon.ino.bootloader.bin (14864 bytes)
0a8b5720e7b77ff11f1462458c3a509dee79224e5279898f26d6a2e3ae0517b7  HeishaMon.ino.partitions.bin (3072 bytes)
78c4be5d9f8fe7f2979b5aa5411e130fa5716e8f322ef9e534ef625e4e99bdbf  littlefs.img (131072 bytes; config.json + rules.txt)
```

Host: x86_64 Linux, 4 cores, Rust stable, esp32sim release build. The run is cycle-deterministic: the
same scenario twice produces the same frame and byte counts (the numbers above are from that pair).

## Limits

- The MQTT evidence needs a broker reachable from the host at the address in the guest's
  `config.json`. The NAT opens a host socket for that exact address, so `127.0.0.1` in the guest never
  reaches it (lwIP routes it to its own loopback: `0 TCP connections`); a test bench uses the host's
  LAN address instead.
- Only the ESP32-S3 MAC gained the hand-over. The C3, C6 and classic models have their own copies of
  this path and were not changed: no specimen there starves its descriptor list, so a change would be
  unverified on those chips. If one does, the same rule applies.
- The restart is triggered by the *stopped* state, not by the base write alone: rewinding a running DMA
  is still wrong (see `docs/decisions.md`), and the second unit test pins that.
- This says nothing about throughput: one frame per `RX_GAP_US` with a 50 ms stall window when software
  is slow to recycle, as before.
