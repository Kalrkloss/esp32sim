#!/usr/bin/env python3
"""The first-boot flow of a firmware configured through its own soft AP, in one emulator run.

The emulator is both radio roles: it joins the access point the guest brings up (`--wifi-join`) and
offers one the guest can see and join (`--wifi`). The run:

1. boots the guest and waits for its setup UI through the forward to the guest's access point,
2. asks the guest to scan — `WIFI.SCAN` / `WIFI.NETWORKS(A$)` over its own WebSocket console, the
   same scan its setup page is built on — and reads the list off the guest's own console,
3. reads the configuration the page would read, sets the station name and passphrase, saves it
   (`POST /_kfg_`) and reboots the guest,
4. waits for the guest to come back as a station of the emulated access point, reached through the
   second forward at the station address, and reports what the emulator and the guest said.

Paths come from the environment; nothing about a particular image is baked in.

    APP=app.bin BOOTLOADER=bootloader.bin PTABLE=ptable.bin BOOT_APP0=boot_app0.bin \
    EMU=target/release/esp32sim SSID=bench PSK=bench-password ./guest-flow.py

`--control` leaves `--wifi` out: the guest must then fail cleanly after the reboot, which is what
the same flow did before the two roles could be on the air together. The summary is written as JSON
to `SUMMARY` (default `guest-flow.json`).

The guest's console is the channel its interpreter prints to: `print` output shows up there, not in
the WebSocket text frames, so the script reads the emulator's own log file as the console.
"""
import base64
import gzip
import json
import os
import re
import socket
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

EMU = os.environ.get("EMU", "target/release/esp32sim")
APP = os.environ["APP"]
BOOTLOADER = os.environ["BOOTLOADER"]
PTABLE = os.environ["PTABLE"]
BOOT_APP0 = os.environ.get("BOOT_APP0")
SSID = os.environ.get("SSID", "bench")
PSK = os.environ.get("PSK", "bench-password")
AP_PORT = int(os.environ.get("AP_PORT", "18080"))       # forward to the guest's own access point
STA_PORT = int(os.environ.get("STA_PORT", "18081"))    # forward to the station address
MAX_SECONDS = os.environ.get("MAX_SECONDS", "75")
CONTROL = "--control" in sys.argv
LOG = os.environ.get("LOG", "guest-flow.log")
SUMMARY = os.environ.get("SUMMARY", "guest-flow.json")

result = {"control": CONTROL, "ssid": SSID, "steps": {}, "ok": False}


def say(message):
    print(f"{time.strftime('%H:%M:%S')} {message}", flush=True)


def http(port, path, data=None, timeout=120):
    headers = {}
    if data is not None:
        headers["Content-Type"] = "application/x-www-form-urlencoded"
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            body = r.read()
            if r.headers.get("Content-Encoding", "") == "gzip" or body[:2] == b"\x1f\x8b":
                try:
                    body = gzip.decompress(body)
                except OSError:
                    pass
            return r.status, body
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def wait_http(port, path="/config", limit=1200, step=5):
    start = time.time()
    last = None
    while time.time() - start < limit:
        try:
            status, body = http(port, path, timeout=15)
            if status == 200:
                return body
            last = f"HTTP {status}"
        except Exception as e:                                  # connection refused, timeout …
            last = str(e)
        time.sleep(step)
    raise SystemExit(f"port {port}: {path} never answered ({last})")


# ---- the guest's console, read from the emulator's log ------------------------------------------

class Console:
    """The guest's console as the emulator prints it: everything since the last call."""

    def __init__(self, path):
        self.path = path
        self.pos = 0

    def take(self):
        try:
            with open(self.path, "rb") as f:
                f.seek(self.pos)
                data = f.read()
                self.pos = f.tell()
        except FileNotFoundError:
            return ""
        return data.decode("utf-8", "replace").replace("\r\n", "\n").replace("\r", "\n")

    @staticmethod
    def guest_only(text):
        """The guest's own console lines: the emulator's own reporting removed."""
        return re.sub(r"(?m)^\[(?:emu|wifi|wifi-join|net|nat|hostfwd)\][^\n]*\n?", "", text)

    def wait_for(self, pattern, seconds, what, guest_only=False):
        end = time.time() + seconds
        seen = ""
        while time.time() < end:
            seen += self.take()
            m = re.search(pattern, self.guest_only(seen) if guest_only else seen, re.M)
            if m:
                return m
            time.sleep(2)
        say(f"timed out waiting for {what}; console since then:\n{seen[-2000:]}")
        return None


# ---- the guest's own WebSocket console ---------------------------------------------------------

def ws_connect(port, timeout=180):
    s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    key = base64.b64encode(os.urandom(16)).decode()
    s.sendall((f"GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n"
               f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
               f"Sec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: Output\r\n\r\n").encode())
    head = b""
    while b"\r\n\r\n" not in head:
        chunk = s.recv(4096)
        if not chunk:
            raise SystemExit("no WebSocket handshake answer")
        head += chunk
    if b"101" not in head.split(b"\r\n")[0]:
        raise SystemExit(f"WebSocket handshake refused: {head.split(b'\r\n')[0]!r}")
    s.settimeout(1.0)
    return s


def ws_send(s, text):
    payload = text.encode()
    mask = os.urandom(4)
    n = len(payload)
    header = bytes([0x81])
    if n < 126:
        header += bytes([0x80 | n])
    elif n < 65536:
        header += bytes([0x80 | 126]) + struct.pack(">H", n)
    else:
        header += bytes([0x80 | 127]) + struct.pack(">Q", n)
    s.sendall(header + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))


def ws_recv(s, seconds):
    out = []
    end = time.time() + seconds
    buf = b""
    while time.time() < end:
        try:
            chunk = s.recv(65536)
            if not chunk:
                return out
            buf += chunk
        except socket.timeout:
            continue
        except OSError:
            return out
        while len(buf) >= 2:
            opcode, n = buf[0] & 0x0f, buf[1] & 0x7f
            off = 2
            if n == 126:
                if len(buf) < 4:
                    break
                n = struct.unpack(">H", buf[2:4])[0]
                off = 4
            elif n == 127:
                if len(buf) < 10:
                    break
                n = struct.unpack(">Q", buf[2:10])[0]
                off = 10
            if len(buf) < off + n:
                break
            payload, buf = buf[off:off + n], buf[off + n:]
            if opcode == 1:
                out.append(payload.decode("utf-8", "replace"))
            elif opcode == 8:
                return out
    return out


def repl(s, command, seconds=3):
    ws_send(s, f"cmd:immediate {command}")
    return ws_recv(s, seconds)


def wait_guest_ap(log_path, limit=180):
    """Wait until the guest's own console says its access point is up (192.168.4.1)."""
    start = time.time()
    while time.time() - start < limit:
        try:
            with open(log_path, "rb") as f:
                if b"IP address: 192.168.4.1" in f.read():
                    return True
        except FileNotFoundError:
            pass
        time.sleep(3)
    return False


def scan(port, console, log_path):
    """Ask the guest to scan; the guest's console is reachable before its interpreter is, so retry."""
    if not wait_guest_ap(log_path):
        say("the guest never reported its access point's address; scanning anyway")
    for attempt in (1, 2, 3):
        try:
            return scan_once(port, console)
        except (BrokenPipeError, ConnectionResetError, OSError) as e:
            say(f"scan attempt {attempt} lost the guest's console: {e}")
            time.sleep(10)
    raise SystemExit("the guest's console never took the scan")


def scan_once(port, console):
    """One scan over a fresh console connection."""
    s = ws_connect(port)
    try:
        say("the guest's console answers: " + repr(repl(s, "__ping__", 5)))
        console.take()                                          # drop what came before the scan
        repl(s, "WIFI.SCAN", 3)
        count = None
        deadline = time.time() + int(os.environ.get("SCAN_WAIT", "480"))
        while time.time() < deadline:
            repl(s, "print WIFI.NETWORKS(A$)", 1)
            found = console.wait_for(r"^(-?\d+)$", 10, "the scan count", guest_only=True)
            if found is None:
                continue
            count = int(found.group(1))
            say(f"WIFI.NETWORKS(A$) = {count}")
            if count >= 0:
                break
        if count is None or count < 0:
            raise OSError(f"no scan count from the guest's console (last {count!r})")
        repl(s, "print A$", 3)
        listing = []
        deadline = time.time() + 120
        while time.time() < deadline:
            for line in console.take().splitlines():
                if re.match(r"^[^,\s][^,]*,[0-9A-Fa-f:]{17},-?\d+,\d+$", line.strip()):
                    listing.append(line.strip())
            if len(listing) >= (count or 0) or (listing and count == 0):
                break
            time.sleep(2)
        return count, listing
    finally:
        s.close()


def reboot(port, console):
    """Ask the guest to restart, the way its config page does; say which request it honoured."""
    for path in ("/?restart=yes", "/config?restart=yes"):
        try:
            http(port, path, timeout=30)
        except Exception as e:
            say(f"{path}: {e}")
        if console.wait_for(r"ESP-ROM:esp32", int(os.environ.get("RESTART_WAIT", "150")), f"the guest to reboot after {path}"):
            return path
        say(f"{path} did not restart the guest")
    return None


# ---- the run ----------------------------------------------------------------------------------

def emulator_command():
    command = [EMU, "--chip", "s3", "--boot", "rom",
               "--bootloader", BOOTLOADER, "--ptable", PTABLE, "--app", APP,
               "--flash-mb", os.environ.get("FLASH_MB", "16"), "--psram-mb", os.environ.get("PSRAM_MB", "8"),
               "--board", os.environ.get("BOARD", "none"), "--console", os.environ.get("CONSOLE", "uart0"),
               "--max-seconds", MAX_SECONDS, "--debug", "wifi-frames"]
    if BOOT_APP0:
        command += ["--flash-at", f"0xe000={BOOT_APP0}"]
    if not CONTROL:
        command += ["--wifi", f"ssid={SSID},chan=6,psk={PSK}"]
    command += ["--wifi-join", os.environ.get("JOIN", ""),
                "--hostfwd", f"tcp:127.0.0.1:{AP_PORT}-192.168.4.1:80"]
    if not CONTROL:
        # The station address only exists on the access point's network, which the control run
        # leaves out.
        command += ["--hostfwd", f"tcp:127.0.0.1:{STA_PORT}-10.0.2.15:80"]
    return command


def main():
    log = open(LOG, "wb")
    command = emulator_command()
    say("emulator: " + " ".join(command))
    log.write(("$ " + " ".join(command) + "\n").encode())
    log.flush()
    proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
    console = Console(LOG)
    try:
        say("waiting for the guest's setup UI through the forward to its access point")
        page = wait_http(AP_PORT)
        text = page.decode("utf-8", "replace")
        result["steps"]["guest_ui"] = {"bytes": len(page), "has_station_fields": "p_staname" in text.lower()}
        say(f"the guest's UI answers ({len(page)} bytes)")

        count, listing = scan(AP_PORT, console, LOG)
        listed = [line for line in listing if line.split(",")[0] == SSID]
        result["steps"]["scan"] = {"count": count, "listing": listing, "lists_ssid": bool(listed)}
        say(f"the guest's scan found {count} network(s): {listing}")
        if listed:
            fields = listed[0].split(",")
            result["steps"]["scan"]["ssid"], result["steps"]["scan"]["bssid"] = fields[0], fields[1]
            result["steps"]["scan"]["rssi"], result["steps"]["scan"]["channel"] = int(fields[2]), int(fields[3])

        current = http(AP_PORT, "/_kfg_", b"")[1].decode("utf-8", "replace")
        params = dict(urllib.parse.parse_qsl(current, keep_blank_values=True))
        say(f"the guest's configuration page reads {len(params)} parameters")
        params["STANAME"] = SSID
        params["STAPASS"] = PSK
        body = urllib.parse.urlencode(params, safe=", -").encode()
        status, echoed = http(AP_PORT, "/_kfg_", body)
        result["steps"]["save"] = {"status": status, "params": len(params),
                                   "echoes_station": SSID in echoed.decode("utf-8", "replace")}
        say(f"POST /_kfg_ (station name and passphrase) -> {status}")
        if status != 200:
            raise SystemExit(f"the save was refused: HTTP {status}")

        which = reboot(AP_PORT, console)
        result["steps"]["restart"] = {"request": which}
        if which is None:
            raise SystemExit("the guest never restarted")
        say(f"the guest restarted after {which}")

        attached = console.wait_for(r"^\[wifi\] WPA2 four-way handshake complete$", 600,
                                    "the guest's four-way handshake with the emulated access point")
        result["steps"]["association"] = {"handshake": bool(attached)}
        if attached:
            say("the guest completed WPA2-PSK with the emulated access point")

        if CONTROL:
            say("control: no access point was offered, so the guest cannot come back as a station")
            time.sleep(int(os.environ.get("CONTROL_WAIT", "300")))
            result["steps"]["station"] = {"answered": False}
            result["ok"] = True
        else:
            say("waiting for the guest as a station of the emulated access point")
            page = wait_http(STA_PORT, limit=900, step=5)
            result["steps"]["station"] = {"answered": True, "bytes": len(page)}
            say(f"the station's UI answers through the forward at 10.0.2.15 ({len(page)} bytes)")
            result["ok"] = bool(result["steps"]["scan"].get("lists_ssid")) and bool(attached)
            say("waiting for the emulator's own end-of-run report (--max-seconds)")
            try:
                proc.wait(timeout=int(os.environ.get("REPORT_WAIT", "900")))
            except subprocess.TimeoutExpired:
                say("the emulator did not reach --max-seconds in time")
    except SystemExit as e:
        say(f"FAILED: {e}")
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                proc.kill()
        log.close()
        with open(SUMMARY, "w") as f:
            json.dump(result, f, indent=2)
    say(f"log: {LOG}, summary: {SUMMARY}, ok: {result['ok']}")
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
