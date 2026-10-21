#!/usr/bin/env python3
"""Drive a soft-AP guest's own web UI to configure its SPI panel, draw on it, and let the run finish.

The guest boots as an access point; the emulator joins it (`--wifi-join ''`) and `--hostfwd` carries
the host's HTTP and WebSocket traffic to `192.168.4.1`. The flow is the one a person at a browser
performs: fetch the page, POST its form, restart into the configured module, then use the guest's own
console to draw. Everything the panel model saw is in the emulator's exit report and in `--tft-png`.

    FW_APP=app.bin FW_BOOTLOADER=bootloader.bin FW_PTABLE=ptable.bin FW_BOOT_APP0=boot_app0.bin \
    ROM=esp32s3_rev0_rom.elf ./probe-panel.py <emulator> <outdir>

The console commands are passed with `--commands 'a|b|c'` (one line each, as typed).
"""
import argparse
import os
import pty
import re
import signal
import socket
import struct
import subprocess
import threading
import time
import urllib.request

CONFIG = ("MODULE=1&DISPLAY=4&ORIENTATION=1&SDCARD=0"
          "&PINS=1,-1,2,41,40,-1,-1,-1,-1,-1,-1,-1")   # the board's MOSI, SCLK, CS, DC


def http(port, path, data=None, timeout=120):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data,
                                 headers={"Content-Type": "application/x-www-form-urlencoded"} if data else {})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read()


def wait_http(port, path="/config", limit=900, step=2):
    start = time.time()
    while time.time() - start < limit:
        try:
            return http(port, path, timeout=10)[0], time.time() - start
        except Exception:
            time.sleep(step)
    raise SystemExit(f"{path} never answered on {port}")


def connect_ws(port, timeout=120):
    s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    s.sendall((f"GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n"
               f"Connection: Upgrade\r\nSec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==\r\n"
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


def send_text(s, text):
    payload = text.encode()
    mask = os.urandom(4)
    header = bytes([0x81])
    if len(payload) < 126:
        header += bytes([0x80 | len(payload)])
    else:
        header += bytes([0x80 | 126]) + struct.pack(">H", len(payload))
    s.sendall(header + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))


def recv_frames(s, seconds, out):
    """Collect server text frames for `seconds` of wall clock (the guest runs slower than real time)."""
    end = time.time() + seconds
    buf = b""
    while time.time() < end:
        try:
            chunk = s.recv(65536)
            if not chunk:
                return
            buf += chunk
        except socket.timeout:
            continue
        while True:
            if len(buf) < 2:
                break
            opcode, n, off = buf[0] & 0x0f, buf[1] & 0x7f, 2
            if n == 126:
                if len(buf) < 4:
                    break
                n, off = struct.unpack(">H", buf[2:4])[0], 4
            elif n == 127:
                if len(buf) < 10:
                    break
                n, off = struct.unpack(">Q", buf[2:10])[0], 10
            if len(buf) < off + n:
                break
            payload, buf = buf[off:off + n], buf[off + n:]
            if opcode == 8:
                return
            if opcode == 1 and payload:
                out.append(payload.decode("utf-8", "replace"))


def console(port, commands, pause=2.0):
    s = connect_ws(port)
    out = []
    send_text(s, "__ping__")
    for command in commands:
        send_text(s, f"cmd:immediate {command}")
        recv_frames(s, pause, out)
        print(f"--- {command}", flush=True)
        for msg in out:
            print(msg[:800], flush=True)
        out.clear()
    s.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("emulator")
    ap.add_argument("outdir")
    ap.add_argument("--seconds", type=int, default=12)
    ap.add_argument("--port", type=int, default=18090)
    ap.add_argument("--commands", default="")
    ap.add_argument("--debug", default="spi")
    args = ap.parse_args()
    os.makedirs(args.outdir, exist_ok=True)
    log_path = os.path.join(args.outdir, "run.log")
    png_path = os.path.join(args.outdir, "frame.png")
    heat_path = os.path.join(args.outdir, "regstat.txt")

    cmd = [args.emulator, "--chip", "s3", "--boot", "rom", "--rom", os.environ["ROM"],
           "--bootloader", os.environ["FW_BOOTLOADER"], "--ptable", os.environ["FW_PTABLE"],
           "--app", os.environ["FW_APP"], "--flash-at", f"0xe000={os.environ['FW_BOOT_APP0']}",
           "--flash-mb", "16", "--psram-mb", "8", "--board", "atech14", "--console", "uart0",
           "--max-seconds", str(args.seconds), "--tft-png", png_path, "--regstat", heat_path,
           "--wifi-join", "", "--hostfwd", f"tcp:127.0.0.1:{args.port}-192.168.4.1:80"]
    if args.debug:
        cmd += ["--debug", args.debug]
    print("$", " ".join(cmd), flush=True)

    log = open(log_path, "w", encoding="utf-8", errors="replace")
    master, slave = pty.openpty()
    proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=slave, stderr=slave, start_new_session=True)
    os.close(slave)

    def pump():
        buf = b""
        while True:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                log.write(line.decode("utf-8", "replace").rstrip("\r") + "\n")
                log.flush()

    threading.Thread(target=pump, daemon=True).start()
    try:
        status, waited = wait_http(args.port)
        print(f"/config answered: {status} after {waited:.0f}s wall", flush=True)
        print("POST /_kfg_:", http(args.port, "/_kfg_", CONFIG.encode())[0], flush=True)
        try:
            print("restart:", http(args.port, "/config?restart=yes", timeout=20)[0], flush=True)
        except Exception as e:
            print("restart:", e, flush=True)
        time.sleep(1)
        status, waited = wait_http(args.port)
        print(f"guest's server back: {status} after {waited:.0f}s wall", flush=True)
        print("read back:", http(args.port, "/_kfg_", b"")[1].decode("utf-8", "replace")[:200], flush=True)
        if args.commands:
            try:
                console(args.port, args.commands.split("|"))
            except Exception as e:
                print("console ended:", repr(e), flush=True)
        try:
            proc.wait(timeout=1800)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
    finally:
        if proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
        time.sleep(0.3)
        log.close()

    text = open(log_path, encoding="utf-8", errors="replace").read()
    print("\n=== report", flush=True)
    for line in text.splitlines():
        if line.startswith("[emu] tft") or line.startswith("[emu] stop"):
            print(line, flush=True)
    for line in text.splitlines():
        if "[emu]" not in line and "[wifi]" not in line and re.search(r"module|ERR|Display", line):
            print(line, flush=True)
    if os.path.exists(heat_path):
        rows = [l for l in open(heat_path).read().splitlines() if "SPI2+" in l]
        print(f"=== {len(rows)} SPI2 register rows in {heat_path}; "
              f"{sum(1 for l in text.splitlines() if l.startswith('[spi2] transfer'))} transactions", flush=True)
    print("frame:", png_path, os.path.getsize(png_path) if os.path.exists(png_path) else "missing", flush=True)


if __name__ == "__main__":
    main()
