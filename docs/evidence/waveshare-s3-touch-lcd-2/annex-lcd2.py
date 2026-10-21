#!/usr/bin/env python3
"""Annex 2.99.27 on the emulated Waveshare ESP32-S3-Touch-LCD-2.

Phase A boots the image, joins its soft AP through --wifi-join, reads the settings record from
`/_kfg_` (an empty POST is the read), writes MODULE=CUSTOM, the display preset, the panel pins,
the touch pins and the CST816 touch controller, saves and restarts.

Phase B boots again from the same flash file, lets the firmware draw its splash and writes
`--tft-png`. It also reports the board's own counter line.

Usage: annex-lcd2.py [--display 89] [--touch 5] [--orientation 0] [--phase A|B|both]
"""

import argparse
import gzip
import os
import pty
import re
import socket
import struct
import subprocess
import threading
import time
import zlib

HERE = "/root/annex-basic"
ROOT = "/root"
EMU = "/root/esp32sim-lcd2/target-bench/release/esp32sim"
APP = f"{HERE}/Annex32-S3_2.99.27_qio_qspi.bin"
FLASH = "/tmp/annex-lcd2.img"
LOG = "/tmp/annex-lcd2.log"


def http(port, method, path, body=b"", ctype="text/plain", timeout=60.0):
    s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    try:
        req = (f"{method} {path} HTTP/1.1\r\nHost: 192.168.4.1\r\nAccept-Encoding: identity\r\n"
               f"Content-Type: {ctype}\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n").encode() + body
        s.sendall(req)
        buf = b""
        while b"\r\n\r\n" not in buf:
            b = s.recv(65536)
            if not b:
                break
            buf += b
        head, _, payload = buf.partition(b"\r\n\r\n")
        st = re.match(rb"HTTP/1\.[01] (\d{3})", head)
        st = int(st.group(1)) if st else None
        cl = re.search(rb"(?i)content-length:\s*(\d+)", head)
        if cl:
            need = int(cl.group(1))
            while len(payload) < need:
                b = s.recv(65536)
                if not b:
                    break
                payload += b
            payload = payload[:need]
        if re.search(rb"(?i)content-encoding:\s*gzip", head) or payload[:2] == b"\x1f\x8b":
            try:
                payload = gzip.decompress(payload)
            except OSError:
                pass
        return st, head, payload
    finally:
        try:
            s.close()
        except OSError:
            pass


def run(cmd, log_path):
    log = open(log_path, "w", encoding="utf-8", errors="replace")
    master, slave = pty.openpty()
    p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=slave, stderr=slave)
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
                log.write(line.decode("utf-8", "replace") + "\n")
                log.flush()

    threading.Thread(target=pump, daemon=True).start()
    return p, log


def bbox(raw, w, h):
    """Non-zero bounding box of an 8-bit RGB PNG (filter byte per row)."""
    stride = w * 3 + 1
    x0 = y0 = None
    x1 = y1 = -1
    for y in range(h):
        row = raw[y * stride + 1:(y + 1) * stride]
        xs = [x for x in range(w) if any(row[x * 3:x * 3 + 3])]
        if xs:
            y0 = y if y0 is None else y0
            y1 = y
            x0 = xs[0] if x0 is None else min(x0, xs[0])
            x1 = max(x1, xs[-1])
    return None if x0 is None else (x0, y0, x1, y1)


def base_cmd(extra):
    return [
        EMU, "--chip", "s3", "--boot", "rom",
        "--rom", f"{ROOT}/esp32sim/roms/esp32s3_rev0_rom.elf",
        "--bootloader", f"{HERE}/bootloader_opi_80m.bin",
        "--ptable", f"{HERE}/ptable-16mb.bin",
        "--app", APP,
        "--flash-at", f"0xe000={HERE}/boot_app0.bin",
        "--flash-mb", "16", "--psram-mb", "8", "--board", "waveshare-s3-touch-lcd-2",
        "--console", "usb", "--no-dump",
    ] + extra


def phase_a(port, display, touch, orientation, pins, i2cport, restart=True):
    cmd = base_cmd(["--wifi-join", "",
                    "--hostfwd", f"tcp:127.0.0.1:{port}-192.168.4.1:80",
                    "--flash-file", FLASH, "--max-seconds", "120"])
    p, log = run(cmd, LOG)
    for _ in range(120):
        try:
            st, _, _ = http(port, "GET", "/config", timeout=5)
            if st:
                break
        except OSError:
            pass
        time.sleep(1)
    else:
        print("phase A: web UI never answered")
        p.terminate()
        return False

    st, _, body = http(port, "POST", "/_kfg_", b"", "application/x-www-form-urlencoded")
    text = body.decode("utf-8", "replace").strip()
    print(f"phase A: POST /_kfg_ (read) -> {st}, {len(body)} B")
    print("phase A: record:", text[:600])
    settings = {}
    for part in re.split(r"&", text):
        if "=" in part:
            k, v = part.split("=", 1)
            settings[k.strip().upper()] = v.strip()

    have = [p for p in settings.get("PINS", "").split(",") if p.strip() != ""]
    n = len(have) if have else 20
    want = pins[:n] + ["-1"] * (n - len(pins))
    settings["MODULE"] = "1"
    settings["DISPLAY"] = str(display)
    settings["TOUCH"] = str(touch)
    settings["ORIENTATION"] = str(orientation)
    settings["PINS"] = ",".join(want) + ","
    if i2cport is not None:
        settings["I2CPORT"] = str(i2cport)
    body_out = "&".join(f"{k}={v}" for k, v in settings.items()).encode()
    st, _, resp = http(port, "POST", "/_kfg_", body_out, "application/x-www-form-urlencoded")
    echo = {}
    for part in resp.decode("utf-8", "replace").split("&"):
        if "=" in part:
            k, v = part.split("=", 1)
            echo[k.strip().upper()] = v.strip()
    print(f"phase A: write DISPLAY={display} TOUCH={touch} MODULE=1 -> {st}")
    print(f"phase A: read-back DISPLAY={echo.get('DISPLAY')} TOUCH={echo.get('TOUCH')} MODULE={echo.get('MODULE')} "
          f"ORIENTATION={echo.get('ORIENTATION')} I2CPORT={echo.get('I2CPORT')}")
    print(f"phase A: read-back PINS={echo.get('PINS')}")

    if restart:
        try:
            http(port, "GET", "/config?restart=yes", timeout=10)
        except OSError:
            pass
        time.sleep(12)
    p.terminate()
    try:
        p.wait(timeout=20)
    except subprocess.TimeoutExpired:
        p.kill()
    log.close()
    return os.path.exists(FLASH)


def phase_b(display, png, seconds, script=None):
    extra = ["--flash-file", FLASH, "--tft-png", png, "--max-seconds", str(seconds)]
    if script:
        extra += ["--script", script]
    p, log = run(base_cmd(extra), LOG.replace(".log", f"-b{display}.log"))
    try:
        p.wait(timeout=seconds + 120)
    except subprocess.TimeoutExpired:
        p.terminate()
        p.wait(timeout=20)
    log.close()
    txt = open(LOG.replace(".log", f"-b{display}.log"), encoding="utf-8", errors="replace").read()
    stats = None
    if os.path.exists(png):
        d = open(png, "rb").read()
        w, h = struct.unpack(">II", d[16:24])
        idat = b""
        i = 8
        while i < len(d):
            ln = struct.unpack(">I", d[i:i + 4])[0]
            if d[i + 4:i + 8] == b"IDAT":
                idat += d[i + 8:i + 8 + ln]
            i += 12 + ln
        raw = zlib.decompress(idat)
        nz = sum(1 for b in raw if b)
        stats = (w, h, nz, len(raw))
        stats = stats + (bbox(raw, w, h),)
    return stats, txt


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--display", type=int, default=89)
    ap.add_argument("--touch", type=int, default=5)
    ap.add_argument("--orientation", type=int, default=0)
    ap.add_argument("--port", type=int, default=18120)
    ap.add_argument("--seconds", type=float, default=20)
    ap.add_argument("--phase", default="both")
    ap.add_argument("--script")
    ap.add_argument("--keep-flash", action="store_true")
    ap.add_argument("--no-restart", action="store_true")
    args = ap.parse_args()

    pins = ["38", "40", "39", "45", "42", "-1", "1", "-1", "-1", "-1", "-1", "-1", "48", "47"]
    if args.phase in ("A", "both"):
        if os.path.exists(FLASH) and not args.keep_flash:
            os.remove(FLASH)
        ok = phase_a(args.port, args.display, args.touch, args.orientation, pins, 0, restart=not args.no_restart)
        print(f"phase A done (flash file: {ok})")
        if not ok:
            return 1
    if args.phase in ("B", "both"):
        png = f"/tmp/annex-lcd2-{args.display}.png"
        if os.path.exists(png):
            os.remove(png)
        stats, txt = phase_b(args.display, png, args.seconds, args.script)
        for line in txt.splitlines():
            if re.search(r"\[emu\]|Annex|assert|Guru|rst:0x|Rebooting|IP address|display res|I2C PORT|module Type", line):
                print("   ", line.strip()[:160])
        if stats:
            w, h, nz, total, box = stats
            print(f"DISPLAY={args.display}: {w}x{h} non-zero={nz}/{total} bbox={box} -> {png}")
        else:
            print(f"DISPLAY={args.display}: no screenshot")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
