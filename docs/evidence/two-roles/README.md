# Both radio roles at once

The emulator offers an access point the guest can scan for and join (`--wifi`) *and* joins the
access point the guest brings up (`--wifi-join`), in the same run. That is the first-boot flow of
firmware that is configured through its own soft AP: the setup page scans, the user picks a network
from the list, saves it and the guest restarts as that network's station — and with one role at a
time the scan lists nothing and the restart ends the test.

## Mechanism

`esp-soc/src/wifi.rs` holds two links and two networks behind one radio:

- `ap: Option<(VirtualAp, VirtualNet)>` — the access point, with the network that serves DHCP, ARP,
  DNS and NTP to the guest as a station (10.0.2.15).
- `join: Option<(Supplicant, VirtualNet)>` — the join to the guest's own access point, with a
  *client* network: it takes a lease from the guest's DHCP server and only the forward's flows are
  answered on it.

One network cannot serve both roles: a guest that restarts into station mode takes its address from
the **access point's** network, while the earlier half of the run needed the **join's** client role.

Addresses decide where a frame goes. A frame the guest transmits belongs to the access point when
addr1 (or addr3, for a data frame to the DS) is the emulated BSSID, and a **broadcast probe request**
belongs to it as well — that is the frame a scan needs answered; everything else the guest transmits
as an access point belongs to the join. Received frames from both links are merged into the single
receive ring in emulated time, one airtime gap apart, because there is one radio.

The receive header's filter-match bits follow the **guest's own mode**, not the frame's link. While
the guest's access point beacons — it is the access point the join is attached to, or the frame came
over that link — its MAC reports a match by address alone (bit 29, without bit 28), so a scan in its
setup UI accepts the emulated access point's probe responses and beacons. After the restart the join
hears no access point any more and the station convention (bit 28 for group traffic, 28|29 for its own
unicast) applies again. `StationLink::rx_match` is where that is decided.

`--hostfwd` specifications are grouped by the guest address they name: one inside the access point's
subnet (the station default 10.0.2.15 included) rides the access point's network, anything else rides
the join's, and a specification that names no address goes to the station — or to the join's peer when
there is no access point. `StationLink::net_for_guest` is that address-to-network map.

The end-of-run report keeps the roles apart: `[emu] wifi:` carries the access point's counters,
`[emu] wifi-join:` the join's, and each network gets its own line (`[emu] net:` for the access point,
`[emu] net (join):` for the join's when both are up). Station-only and join-only runs print what they
printed before.

## Checks

```sh
# the two roles, the routing, the scan answer and the two networks
cargo test -p esp-soc --release wifi::two_roles
# everything CI runs
cargo clippy --workspace --all-targets -- -D warnings
ESP32SIM_ROM_DIR=<roms> cargo test --release --workspace -- --include-ignored --skip external_
```

The unit tests drive both halves with real frames: the guest's own access point is the model's
`VirtualAp`, so the scan answer is built and received the way the air would carry it. Measured here:
both Clippy runs clean with warnings denied, and the workspace suite (ignored tests included,
`external_*` skipped) 994 tests in 102 binaries, 0 failures.

## The run

Inputs (a released soft-AP image, a BASIC interpreter with a web configuration page, kept in a local
directory on the machine that ran it and quoted here with neutral names):

| input | sha256 |
| --- | --- |
| `app.bin` | `645a7e932be51aac212081890ece17ef56fceee829c2e060883525b942389109` |
| `bootloader.bin` | `7c2892c8bea5762e3c9f44cfbae154d01f2da93dea97f9238fea4b02a08d35ed` |
| `ptable.bin` | `24db2c95993e99f85fb925b8d8d722b113e907b1a7d8597656d233415944c5c6` |
| `boot_app0.bin` | `f94c5d786a7a8fab06ac5d10e33bf37711a6697636dc037559ea19cc410a17f0` |
| ROM ELF (`esp32s3_rev0_rom.elf`) | c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd |
| emulator binary (this branch merged with the inbound-forward branch locally, release build) | 1f70ccbacf2e52038a95197f7e37e44a8511fa84da7c583f2950a9fcd20b56db |

```sh
APP=app.bin BOOTLOADER=bootloader.bin PTABLE=ptable.bin BOOT_APP0=boot_app0.bin \
EMU=target/release/esp32sim SSID=bench PSK=bench-pass MAX_SECONDS=60 \
./guest-flow.py
```

`guest-flow.py` starts the emulator with both roles and two forwards (the guest's access point
192.168.4.1:80 on 18080, the station address 10.0.2.15:80 on 18081), waits for the setup UI, asks the
guest to scan over its own WebSocket console (`WIFI.SCAN`, `WIFI.NETWORKS(A$)`, `print A$` — the
guest's interpreter prints to its console, so the list is read from there), reads the configuration
the page would read, sets the station name and passphrase, saves it with `POST /_kfg_` and restarts
the guest on its own restart path. `run-two-roles.sh` is the emulator command line on its own.

## What came out

One run of the image named above, in order (`--debug wifi-frames`; the probe response's timestamp is
elided). The emulated MAC joined the access point the guest brought up, and the guest's UI answered
through the first forward:

```
[emu] both radio roles are on the air: the guest can scan for the emulated access point and join it, while this side is a station of the guest's own access point
[emu] hostfwd: tcp:127.0.0.1:18080 -> 192.168.4.1:80
[emu] hostfwd: tcp:127.0.0.1:18081 -> 10.0.2.15:80
[wifi-join] heard AP 'ESP46:1B:F6:75:DC:E0' bssid 46:1b:f6:75:dc:e0 channel 1 (open)
[wifi-join] associated
[wifi-join] lease 192.168.4.2 mask 255.255.255.0 router 192.168.4.1 dns 192.168.4.1
[wifi-join] joined 'ESP46:1B:F6:75:DC:E0' as 192.168.4.2
```

The guest scanned (`WIFI.SCAN`, then `WIFI.NETWORKS(A$)` until the count is not `-1`, then `print A$`)
and its own console listed the emulated access point with the BSSID, RSSI and channel `--wifi`
configured:

```
bench,02:53:49:4D:00:01,-40,6
```

Every probe request its scan transmitted was answered by the access point role and delivered back into
its receive ring, under the address-match verdict a guest in the AP role accepts:

```
[wifi] AP -> 98 bytes probe-resp (0/5) a1=44:1b:f6:75:dc:e0 a2=02:53:49:4d:00:01 a3=02:53:49:4d:00:01 ssid='bench'
[wifi] RX -> desc 0x3fcf7c34 98 bytes probe-resp (0/5) a1=44:1b:f6:75:dc:e0 a2=02:53:49:4d:00:01 a3=02:53:49:4d:00:01 ssid='bench'
```

The page's own save answered and the guest restarted — on its own restart path, not on `/?restart=yes`:

```
the guest's configuration page reads 23 parameters
POST /_kfg_ (station name and passphrase) -> 200
/?restart=yes did not restart the guest
the guest restarted after /config?restart=yes
```

It came back as a station of the emulated access point: association, the four-way handshake with the
emulated authenticator verifying the guest's MIC, then its own DHCP lease from the access point's
network — while the join link, whose access point no longer beacons, gave up cleanly:

```
[wifi] RX AUTH/ASSOC -> desc 0x3fcf7d5c buf 0x3fca6c6c 46 bytes assoc-resp (0/1) a1=44:1b:f6:75:dc:e0 a2=02:53:49:4d:00:01 a3=02:53:49:4d:00:01
[wifi] WPA2 msg2: PTK derived, station MIC VERIFIED (recv [7e, d6, 8f, 08] calc [7e, d6, 8f, 08])
[wifi] WPA2 four-way handshake complete
Connected to bench
IP address: 10.0.2.15
```

and the second forward reached its web UI at the station address (12 833 bytes, the same page). The
emulator's end-of-run report keeps the roles and the networks apart:

```
[emu] stop: Halted — core0 248715839 + core1 12784527561 insns in 1295.6s wall = 10.1 Minsn/s; emulated 60.000s (14400000000 cycles); 1178428 exceptions, 129787 interrupts
[emu] wifi: 43 frames sent by the station, 825 received (7 dropped: no descriptor); AP: 585 beacons, 33 probe responses, 14 data frames from the station, state Associated
[emu] wifi-join: AP -, state Failed, lease -, 46 beacons (46 matched), 304 probes, 1 auths, 1 assocs, 0 EAPOL-2, 0 EAPOL-4, keys false, 2 DHCP requests, 1 ARP, 61 frames from the guest, 209 to it, failed: no matching access point on the air
[emu] net: 1 DHCP leases, 3 ARP replies, 0 DNS answers, 0 NTP answers, 0 TCP refused, 0 pings, 0 frames ignored
[emu] nat: 0 TCP connections (0 failed), 2 UDP flows (0 evicted, 0 send errors), 78 bytes out, 142 bytes in
[emu] hostfwd: 1 host connections (1 established, 0 refused by the guest, 0 refused for want of a slot, 0 abandoned), 1 closed, 0 active, 0 in TIME_WAIT, 0 segments without a flow, 125 bytes in, 4385 bytes out
[emu] net (join): 0 DHCP leases, 0 ARP replies, 0 DNS answers, 0 NTP answers, 0 TCP refused, 0 pings, 2 frames ignored
[emu] nat (join): 0 TCP connections (0 failed), 0 UDP flows (0 evicted, 0 send errors), 0 bytes out, 0 bytes in
[emu] hostfwd: 7 host connections (7 established, 0 refused by the guest, 0 refused for want of a slot, 0 abandoned), 7 closed, 0 active, 0 in TIME_WAIT, 0 segments without a flow, 6910 bytes in, 8643 bytes out
```

The access point's network is the one that leased the station its address (`1 DHCP leases`); the
join's network answered none of it (`0 DHCP leases`, and the two frames it ignored are the guest's
earlier broadcasts). Each forward rode its own link: seven host connections to the guest as an access
point, one to it as a station. The join reporting `no matching access point on the air` is the
restart, not a fault — the guest stopped beaconing when it became a station, and the join says so
instead of waiting.

Control run (`--control`: no `--wifi`, so only the join role; the station forward is left out because
its address only exists on the access point's network). The scan finds nothing at all —

```
the guest's scan found 0 network(s): []
```

— the save still answers 200, and after the restart the guest cannot reach `bench`: its console prints
`Connecting to bench`, it keeps running, brings its own access point back (the join reports
`state Online, lease 192.168.4.2` again), and the run reaches `--max-seconds` and exits normally:

```
[emu] stop: Halted — core0 156677190 + core1 8366121142 insns in 510.5s wall = 16.7 Minsn/s; emulated 60.000s (14400000000 cycles); 572704 exceptions, 132357 interrupts
[emu] wifi: 348 frames sent by the station, 254 received (2 dropped: no descriptor)
[emu] wifi-join: AP 'ESP46:1B:F6:75:DC:E0' bssid 46:1b:f6:75:dc:e0 channel 1, state Online, lease 192.168.4.2 via 192.168.4.1, 288 beacons (288 matched), 256 probes, 2 auths, 2 assocs, 0 EAPOL-2, 0 EAPOL-4, keys false, 4 DHCP requests, 1 ARP, 67 frames from the guest, 202 to it
[emu] net: 0 DHCP leases, 0 ARP replies, 0 DNS answers, 0 NTP answers, 0 TCP refused, 0 pings, 4 frames ignored
[emu] nat: 0 TCP connections (0 failed), 0 UDP flows (0 evicted, 0 send errors), 0 bytes out, 0 bytes in
[emu] hostfwd: 7 host connections (7 established, 0 refused by the guest, 0 refused for want of a slot, 0 abandoned), 7 closed, 0 active, 0 in TIME_WAIT, 0 segments without a flow, 6523 bytes in, 8643 bytes out
```

No `AP:` counters, no station address, and nothing hung: without the access point role the guest ends
in its own AP mode, which the join re-joins.

## Limits

- **`/?restart=yes` does not restart this image.** It answers and keeps running; its configuration
  page posts `/config?restart=yes`, and that is what the script uses after trying the other one. The
  attempt and its outcome are in the log.
- **The scan is slow in wall-clock terms.** This image runs at ~0.2× real time and its scan takes
  ~17 emulated seconds, so the list arrives about 85 s after `WIFI.SCAN`. That is the image, not the
  link: the emulator answers every probe request the moment it is transmitted.
- **The forward's per-address attachment lives on the `--hostfwd` branch.** This branch adds
  `StationLink::net_for_guest` and the CLI's both-flags handling; the change that groups the
  specifications by guest address, attaches each group to its network (`attach_hostfwd_for`) and sets
  the join's peer and guest addresses from the lease is on the inbound-forward branch, whose tree the
  run above used (a local merge, not for pushing). Both branches touch the same few lines of
  `net_step` and the same setup functions in `cli/src/lib.rs`, so one of them lands first and the
  other keeps that small diff.
- **Same picture as the join: no 802.11n/ac, no channel switching, no TKIP, PMF/802.11w.** The C3,
  C6 and classic ESP32 compile against the same link code but were not exercised with two roles.
- **Only TCP rides the forward.** The join link carries ARP, ICMP, DHCP and EAPOL itself; the
  access point's network carries DHCP, ARP, DNS, NTP and the NAT.
