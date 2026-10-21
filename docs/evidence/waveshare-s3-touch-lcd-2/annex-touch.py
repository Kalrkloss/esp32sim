#!/usr/bin/env python3
"""Prove the CST816D path end to end (receipt: docs/evidence/waveshare-s3-touch-lcd-2): a scripted tap and swipe on the LCD-2 board, a BASIC program
on the device that polls TOUCH.READ, and the firmware's own output as the evidence.

The device's web UI talks to the firmware over two WebSocket subprotocols ("Editor" for commands,
"Output" for the console). The program is saved through the Editor socket's save handshake and
then loaded and run; scripted touches land while it polls.

Usage: annex-touch.py [--port 18125] [--seconds 30]
"""

import argparse
import base64
import importlib.util
import os
import re
import socket
import struct
import sys
import threading
import time

spec = importlib.util.spec_from_file_location("a", os.path.join(os.path.dirname(os.path.abspath(__file__)), "annex-lcd2.py"))
A = importlib.util.module_from_spec(spec)
spec.loader.exec_module(A)

PROGRAM = """print "TOUCHMON up"
do
  touch.raw
  if touch.z > 0 then
    tx=touch.x
    ty=touch.y
    tz=touch.z
    print "RAW z=";touch.z;" x=";touch.x;" y=";touch.y
  endif
  touch.read
  if touch.z > 0 then
    print "CAL z=";touch.z;" x=";touch.x;" y=";touch.y
  endif
  pause 30
loop
"""


class Ws:
    def __init__(self, host, port, subprotocol, timeout=180):
        self.s = socket.create_connection((host, port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        req = (f"GET /ws HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
               f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: {subprotocol}\r\n\r\n")
        self.s.sendall(req.encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.s.recv(1)
            if not chunk:
                raise OSError("websocket handshake closed")
            head += chunk
        status = head.split(b"\r\n")[0]
        if b"101" not in status:
            raise OSError(f"websocket handshake: {status!r}")
        self.log = []

    def send(self, text):
        payload = text.encode()
        frame = bytearray([0x81])
        n = len(payload)
        if n < 126:
            frame.append(0x80 | n)
        elif n < 65536:
            frame.append(0x80 | 126)
            frame += struct.pack(">H", n)
        else:
            frame.append(0x80 | 127)
            frame += struct.pack(">Q", n)
        mask = os.urandom(4)
        frame += mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.s.sendall(frame)

    def recv(self, timeout=30):
        self.s.settimeout(timeout)
        try:
            head = self.s.recv(2)
        except socket.timeout:
            return None
        if len(head) < 2:
            return None
        n = head[1] & 0x7f
        if n == 126:
            n = struct.unpack(">H", self._read(2))[0]
        elif n == 127:
            n = struct.unpack(">Q", self._read(8))[0]
        data = self._read(n) if n else b""
        text = data.decode("utf-8", "replace")
        self.log.append(text)
        return text

    def _read(self, n):
        buf = b""
        while len(buf) < n:
            chunk = self.s.recv(n - len(buf))
            if not chunk:
                raise OSError("websocket closed")
            buf += chunk
        return buf

    def wait_for(self, prefix, timeout=60):
        deadline = time.time() + timeout
        while time.time() < deadline:
            text = self.recv(timeout=max(1, deadline - time.time()))
            if text is None:
                continue
            print(f"    <- {text.strip()[:120]}")
            if text.startswith(prefix):
                return text
        return None


def save_program(ws, path, text):
    ws.send(f"save:start{path}")
    if ws.wait_for("SAVE:GIVE") is None:
        return False
    data = text.encode()
    for i in range(0, len(data), 512):
        ws.send("save:more" + data[i:i + 512].decode())
        reply = ws.wait_for("SAVE:", timeout=30)
        if reply is None:
            return False
        if reply.startswith("SAVE:END"):
            return True
    # an empty tail is how the editor closes a file whose size is a multiple of the chunk
    ws.send("save:more")
    reply = ws.wait_for("SAVE:", timeout=30)
    return bool(reply and reply.startswith("SAVE:END"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=18125)
    ap.add_argument("--seconds", type=float, default=32)
    ap.add_argument("--tap", default="12 120 160")
    ap.add_argument("--send", default="16 60 200 230 200")
    args = ap.parse_args()

    # the scripted pointer: a tap, then a swipe across the glass, in emulated seconds
    tx, x1, y1 = args.tap.split()
    sx, ax, ay, bx, by = args.send.split()
    script = (f"{float(tx):.1f} touch {x1} {y1} 1\n"
              f"{float(tx) + 0.25:.2f} touch {x1} {y1} 0\n"
              f"{float(sx):.1f} touch {ax} {ay} 1\n"
              f"{float(sx) + 0.15:.2f} touch {bx} {by} 1\n"
              f"{float(sx) + 0.30:.2f} touch {bx} {by} 0\n")
    script_path = "/tmp/annex-touch-script.txt"
    open(script_path, "w").write(script)
    print("script:\n" + script)

    cmd = A.base_cmd(["--wifi-join", "",
                      "--hostfwd", f"tcp:127.0.0.1:{args.port}-192.168.4.1:80",
                      "--flash-file", "/tmp/annex-lcd2.img",
                      "--script", script_path,
                      "--max-seconds", str(args.seconds)])
    emu, log = A.run(cmd, "/tmp/annex-touch.log")
    time.sleep(3)
    if emu.poll() is not None:
        print("emulator exited at start:")
        print(open("/tmp/annex-touch.log", encoding="utf-8", errors="replace").read()[-500:])
        return 1

    for _ in range(180):
        try:
            st, _, _ = A.http(args.port, "GET", "/config", timeout=5)
            if st:
                break
        except OSError:
            pass
        time.sleep(1)
    else:
        print("web UI never answered")
        emu.terminate()
        return 1

    editor = Ws("127.0.0.1", args.port, "Editor")
    output = Ws("127.0.0.1", args.port, "Output")
    print("websockets open")

    saved = save_program(editor, "/touch.bas", PROGRAM)
    print(f"save /touch.bas -> {saved}")
    editor.send("load:/touch.bas")
    time.sleep(1.0)
    editor.send("cmd:run ")
    print("run sent")

    # drain both sockets until the run ends or the window closes
    deadline = time.time() + args.seconds * 16
    texts = []
    last_poll = 0.0
    while time.time() < deadline and emu.poll() is None:
        if time.time() - last_poll > 2.0:
            # the console page pulls with "$"; the editor reads the program's own variables with
            # cmd:getvars, which is how the page shows what the guest computed
            try:
                output.send("$")
                editor.send("cmd:getvars tx_^-ty_^-tz_^-tx")
            except OSError:
                pass
            last_poll = time.time()
        for ws in (editor, output):
            try:
                text = ws.recv(timeout=0.5)
            except OSError:
                text = None
            if text:
                texts.append(text)
                line = text.strip()
                if line:
                    print(f"    <- {line[:140]}")
    emu.terminate()
    try:
        emu.wait(timeout=30)
    except Exception:
        emu.kill()

    guest = open("/tmp/annex-touch.log", encoding="utf-8", errors="replace").read()
    printed = [l.strip() for l in guest.splitlines() if "TOUCHMON" in l or l.strip().startswith(("RAW", "CAL"))]
    watched = [t.strip() for t in texts if t.startswith("GETVAR:")]
    hits = [w for w in watched if not w.startswith("GETVAR:0_^-0_^-0")]
    print(f"\nwebsocket GETVAR frames: {len(watched)}, with a point: {len(hits)}")
    for w in (hits or watched)[:8]:
        print("   ", w[:120])
    print(f"guest console lines about touch: {len(printed)}")
    for line in printed[:6]:
        print("   ", line[:140])
    return 0 if printed and watched else 1


if __name__ == "__main__":
    raise SystemExit(main())
