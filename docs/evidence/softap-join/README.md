# The guest as access point (`--wifi-join`)

**Question.** The model puts the guest on the air as a *station* behind a virtual access point, so
firmware that is configured through its own soft AP — it boots, beacons an AP, and serves its setup
page on `192.168.4.1` — could not be reached at all: nothing on that link is on `10.0.2.0/24`, and
`--hostfwd` refused any guest address but the station's. Can the roles be inverted, so the emulated
MAC joins the access point the guest runs, and can a host connection then reach a server behind it?

**Answer.** Yes for the link and the web UI. `--wifi-join` puts the emulated MAC on the air as a
station of the guest's AP: beacons are heard, open-system authentication and association complete,
the WPA2 four-way handshake runs with the emulator as the *supplicant*, and the guest's own DHCP
server leases an address. On that link `--hostfwd tcp:127.0.0.1:PORT-192.168.4.1:80` carries host
connections to the guest's web server, and a whole configuration flow of a soft-AP firmware ran from
the host with no serial console: its page, its form POST, its restart, the re-join after that
restart, and its own WebSocket console.

**Not answered: the panel frame.** The configuration reached the guest's display driver — the boot
after the form POST reports the module and display the form selected, and the driver then brings up
its SPI bus — but no byte of it ever reached the modelled panel, so `--tft-png` still captures a
black frame. That is a gap in the model's side of the guest's `spi_master` path, not in the join: the
driver configures SPI2 and then never starts a transfer (evidence below). The end-to-end proof of a
*display* configuration therefore stops one layer short of the glass.

## Code

- `esp-soc/src/wifi/join.rs` — `JoinConfig` (the `--wifi-join` grammar) and `Supplicant`: scan, auth,
  association, the four-way handshake in the supplicant role, a DHCP client, ARP/ICMP for the leased
  address, and 802.11 data framing to the DS (protection bit, CCMP header and MIC space once the keys
  are installed).
- `esp-soc/src/wifi.rs` — `StationLink` gains the second link (`attach_join`, `join`, `on_air`,
  `channel`, `log`, `rx_match`); `next_rx`/`nothing_due`/`station_tx` serve whichever side of the air
  exists, and a guest reboot keeps the join and starts it scanning again.
- `esp-soc/src/net.rs` — `VirtualNet` gains a client role: on a join link the emulated subnet's DHCP
  server, gateway, resolver and NTP do not exist, and the only thing left on the host side is a
  forwarded connection (`peer_tcp`).
- `esp-soc/src/hostfwd.rs` — on the inbound-forward branch (EX231, see *Branches*): `Spec` may name a
  guest address, and `HostFwd` learns the peer address and the guest address from the lease.
- `esp-soc/src/soc.rs`, `cli` — `attach_wifi_join` for the four WiFi chips; `--wifi-join SPEC`
  (excludes `--wifi`); on a join link the forward is parsed with no station address.

## Branches

The join stands on its own (`softap-join`, cut from `main`), but the forward it rides is the
inbound-forward change (EX231), which is not on `main`. Every end-to-end run below therefore used a
local merge of the two, `softap-join-bench` = `softap-join` + the `hostfwd` branch + the four
adaptations that only exist there (`Spec::parse` with an unknown guest address, `set_peer_ip`,
`set_guest_ip`, the `peer_tcp` body in `net.rs`). Nothing of that merge is meant to be pushed: the
join is the change under review, and `--hostfwd` keeps its own branch and receipt.

## Measurements

Host: Linux x86_64 (4 cores), `cargo build --release`, no JIT for the S3. Guest firmware
(unmodified, released): app
`645a7e932be51aac212081890ece17ef56fceee829c2e060883525b942389109`, bootloader
`7c2892c8bea5762e3c9f44cfbae154d01f2da93dea97f9238fea4b02a08d35ed`, partition table
`24db2c95993e99f85fb925b8d8d722b113e907b1a7d8597656d233415944c5c6`, `boot_app0`
`f94c5d786a7a8fab06ac5d10e33bf37711a6697636dc037559ea19cc410a17f0`, S3 mask ROM
`esp32s3_rev0_rom.elf` `c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd`; 16 MB
flash, 8 MB PSRAM, `--board atech14` (the ST7735 panel the model has), `--wifi-join ''`.

Speed: 19.1 Minsn/s of emulated execution, i.e. about **0.08×** real time for this image (45.0 s
emulated in 568.7 s wall with the display board, ~9 minutes without it). That rate is the image's,
not the join's or the board's; every run here is budgeted in emulated seconds because of it.

### The join, frame by frame

`--debug wifi-frames`, first 12 emulated seconds:

```
[wifi] TX slot 0 ... 189 bytes beacon (0/8) a1=ff:ff:ff:ff:ff:ff a2=46:1b:f6:75:dc:e0 a3=46:1b:f6:75:dc:e0 ssid='ESP46:1B:F6:75:DC:E0'
[wifi] RX -> desc 0x3fcf7c34 54 bytes probe-req (0/4) a1=ff:ff:ff:ff:ff:ff a2=02:53:49:4d:00:11 a3=ff:ff:ff:ff:ff:ff
[wifi] TX slot 0 ... 178 bytes probe-resp (0/5) a1=02:53:49:4d:00:11 a2=46:1b:f6:75:dc:e0 ...
[wifi-join] heard AP 'ESP46:1B:F6:75:DC:E0' bssid 46:1b:f6:75:dc:e0 channel 1 (open)
[wifi] RX AUTH/ASSOC -> desc ... 30 bytes auth (0/11) a1=46:1b:f6:75:dc:e0 a2=02:53:49:4d:00:11 a3=46:1b:f6:75:dc:e0
[wifi] TX slot 0 ... 34 bytes auth (0/11) a1=02:53:49:4d:00:11 a2=46:1b:f6:75:dc:e0
[wifi] RX AUTH/ASSOC -> desc ... 66 bytes assoc-req (0/0) a1=46:1b:f6:75:dc:e0 a2=02:53:49:4d:00:11 ssid='ESP46:1B:F6:75:DC:E0'
[wifi] TX slot 0 ... 53 bytes assoc-resp (0/1) a1=02:53:49:4d:00:11 a2=46:1b:f6:75:dc:e0
[wifi-join] associated
[wifi-join] DHCP offer 192.168.4.2
[wifi-join] lease 192.168.4.2 mask 255.255.255.0 router 192.168.4.1 dns 192.168.4.1
[wifi-join] joined 'ESP46:1B:F6:75:DC:E0' as 192.168.4.2
```

### The receive-header verdict that made it work

The same image, the same 12 emulated seconds, with the filter bits forced through a temporary hook
(the hook is not part of the change; `StationLink::rx_match` is):

| match bits in `rx_ctrl` word 0 | probe answered | authentication answered | association | DHCP |
| --- | --- | --- | --- | --- |
| 28 + 29 (the station's convention) | no | no | never | — |
| 29 alone | yes | yes | yes | offer and lease |

### The web UI, from the host, through the forward

```
kfg HTTP 200                       # the config form POST, as the guest's page sends it
restart: connection closed         # POST /config?restart=yes: the guest reboots into the new module
the guest's web server is back     # the join re-associates and the lease returns
--- cmd:gethelp
__pong__
HELP_URL:<the guest's own help URL>
```

`GET /config` answers `200` with the gzipped page the guest serves (2 870 bytes `output.html`), the
form POST answers `200` with the configuration echoed back, and the guest's own WebSocket console
answers `__pong__` — three separate channels of the guest's UI, all carried by forwarded connections
over the join link.

### The configuration reaching the display driver, and stopping there

| boot | guest's own report | SPI2 activity | panel |
| --- | --- | --- | --- |
| first, before the form POST | `module Type 0 Display 255 SD_Card 0` | none | 0 RAMWR |
| second, after it | `module Type 1 Display 4 SD_Card 0` | `spi_bus_initialize`: `DMA_CONF` (0x30) 0x60180000, `USER` (0x10) 0x18000001, `W0..W15` cleared, `CLOCK` (0x0c) 0x3c4002 (80 MHz) | 0 RAMWR |

After that initialization the guest never writes `SPI_CMD` (offset 0x00) and never touches SPI3
(0x6002_5000..0x6002_6000, probed for the whole run), so no transfer is ever started and the board's
ST7735 decoder sees nothing: `--tft-png` is 480×240 of black, `[emu] tft: 0 RAMWR, 0 pixels`. The
configuration path works up to the driver; what is missing is the model's side of the transfer
(`spi_master`'s first transaction, most likely its interrupt/DMA completion), which is a separate
piece of work.

### Control: a join that finds nothing must refuse, not hang

`--wifi-join ssid=no-such-access-point`, the same forward, `--board none`, 45 emulated seconds. The
guest beacons throughout; nothing matches, so the join gives up and says why, and the host's
connection is never established — it is held for the station deadline and then closed:

```
client saw: HTTP 000 after 60 s of its own timeout
[emu] wifi-join: AP -, state Failed, lease -, 438 beacons (0 matched), 300 probes, 0 auths, 0 assocs,
  0 EAPOL-2, 0 EAPOL-4, keys false, 0 DHCP requests, 0 ARP, 0 frames from the guest, 0 to it,
  failed: no matching access point on the air
[emu] hostfwd: 1 host connections (0 established, 0 refused by the guest, 0 refused for want of a slot,
  1 abandoned), 0 closed, 0 active, 0 in TIME_WAIT, 0 segments without a flow, 0 bytes in, 0 bytes out
```

`1 abandoned` is the forward's own deadline (`WAIT_STATION_US`, 30 emulated seconds) closing the
connection; the client's 60-second wall-clock timeout expired first, which is a property of this
image's 0.08× speed, not of the forward. Either way nothing hung in the emulator, and no segment was
ever addressed to a peer port.

## Commands

```sh
# emulator: join whatever AP the guest brings up, and carry a host listener to it
esp32sim --chip s3 --boot rom ... --board atech14 \
         --wifi-join '' --hostfwd tcp:127.0.0.1:18080-192.168.4.1:80 --max-seconds 45

# host side: the guest's config page, the form POST its page sends, then its own console
curl -sS http://127.0.0.1:18080/config
curl -sS -X POST -H 'Content-type: application/x-www-form-urlencoded' \
     --data 'MODULE=1&DISPLAY=4&ORIENTATION=1&PINS=1,-1,2,41,40,-1,-1,-1,-1,-1,-1,-1' \
     http://127.0.0.1:18080/_kfg_
# the guest's own WebSocket console (guest-ui.py: handshake, masked text frames)
python3 guest-ui.py 18080 2 --config 'tft.init 1' 'tft.fill 0xF800'
#   --config posts the display form and restarts the guest; then waits for the server to return

# unit tests
cargo test -p esp-soc --release wifi::join
```

[guest-ui.py](guest-ui.py) is that WebSocket/HTTP client and [run-softap.sh](run-softap.sh) is the
emulator command line with the image paths taken from the environment. The guest's console lines are
quoted as they came out, with the product URL in its help answer replaced by a placeholder.

## Limits

- **No panel frame** (above). The display configuration is proven up to the guest's driver, not to
  the glass.
- TCP only, through the forward; the join itself carries ARP, ICMP, DHCP and EAPOL.
- WPA2 is covered by unit tests against the model's own authenticator (`VirtualAp`), not against a
  real blob: the firmware used here brings its AP up open, and its access-point password lives in the
  same form that needs the link to be reachable first.
- Legacy (non-HT) frames only, one BSSID, no channel switching, no PMF/TKIP.
- The guest's own address is the one the forward uses; a second host on the guest's subnet is not
  modelled.
- Only the AP interface can be joined; a guest that runs a station *and* an AP is joined through the
  AP one.
