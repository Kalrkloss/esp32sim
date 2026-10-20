#!/usr/bin/env python3
"""Drive a soft-AP guest's own web UI: configure it over HTTP, then use its WebSocket REPL.

The guest serves a WebSocket (`/ws`, subprotocol `Output`) that carries its console, its editor's
"immediate" channel (`cmd:immediate <line>`) and its runtime commands (`cmd:run`). That is how the
page in a browser talks to it, so this script is the browser: handshake, masked text frames, and
every server frame printed.

usage: guest-ui.py PORT [seconds-between-commands] [--config] command [command ...]

The form in `--config` is the one the guest's config page posts (module CUSTOM, an ST7735 with the
pin set of the board model used in the receipt); change it for another guest.
"""
import base64
import os
import socket
import struct
import sys
import time
import urllib.request

CONFIG = ('MODULE=1&DISPLAY=4&ORIENTATION=1&SDCARD=0'
          '&PINS=1,-1,2,41,40,-1,-1,-1,-1,-1,-1,-1'
          '&APPASS=%s') % os.environ.get("AP_PASS", "")


def http(port, path, data=None, timeout=20):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data,
                                 headers={"Content-Type": "application/x-www-form-urlencoded"} if data else {})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read()


def wait_http(port, path="/config", limit=600, step=2):
    start = time.time()
    while time.time() - start < limit:
        try:
            return http(port, path, timeout=5)[0]
        except Exception:
            time.sleep(step)
    raise SystemExit("the guest's web server never answered")


def connect_ws(port, timeout=120):
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
        raise SystemExit(f"WebSocket handshake refused: {head.split(chr(13).encode())[0]!r}")
    s.settimeout(1.0)
    return s


def send_text(s, text):
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


def recv_frames(s, seconds, out):
    end = time.time() + seconds
    buf = b""
    while time.time() < end:
        try:
            chunk = s.recv(65536)
            if not chunk:
                return False
            buf += chunk
        except socket.timeout:
            continue
        while len(buf) >= 2:
            b0, b1 = buf[0], buf[1]
            opcode, n = b0 & 0x0f, b1 & 0x7f
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
                return False
    return True


def main():
    port = int(sys.argv[1])
    args = sys.argv[2:]
    pause = 1.0
    if args and args[0].replace('.', '', 1).isdigit():
        pause = float(args.pop(0))
    if args and args[0] == "--config":
        print("config:", http(port, "/_kfg_", CONFIG.encode())[0], flush=True)
        try:
            http(port, "/config?restart=yes", timeout=5)
        except Exception as e:
            print("restart:", e, flush=True)
        print("waiting for the guest to boot the configured module", flush=True)
        wait_http(port, limit=900, step=3)
        print("the guest's web server is back", flush=True)
        # let the access point settle so the join has an address for the forward
        time.sleep(3)
        args.pop(0)
    if not args:
        return
    s = connect_ws(port)
    out = []
    send_text(s, "__ping__")
    for cmd in args:
        send_text(s, cmd if cmd.startswith(("cmd:", "$")) else f"cmd:immediate {cmd}")
        recv_frames(s, pause, out)
        print(f"--- {cmd}", flush=True)
        for msg in out:
            print(msg[:2000])
        out.clear()
    recv_frames(s, 2.0, out)
    for msg in out:
        print(msg[:2000])
    s.close()


if __name__ == "__main__":
    main()
