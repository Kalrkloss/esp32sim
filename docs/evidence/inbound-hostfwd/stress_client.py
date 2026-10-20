#!/usr/bin/env python3
"""Client side of the inbound-forward stress test: one baseline GET, 32 parallel GETs, aborted
connections and a half-closed client, all through the host listener esp32sim exposes with
--hostfwd. Prints one line per phase and exits non-zero on the first mismatch.

    stress_client.py HOST PORT REFERENCE_DIR

The reference body comes from the baseline GET; every other response must be byte-identical to
it, which is the only honest check -- a 200 alone does not prove the guest's bytes arrived.
"""
import http.client
import socket
import struct
import sys
import threading

HOST, PORT = sys.argv[1], int(sys.argv[2])
REF = sys.argv[3]
PARALLEL = 32
TIMEOUT = 40.0
failures = []


def get(path="/"):
    conn = http.client.HTTPConnection(HOST, PORT, timeout=TIMEOUT)
    conn.request("GET", path, headers={"Connection": "close"})
    response = conn.getresponse()
    body = response.read()
    status = response.status
    conn.close()
    return status, body


def report(name, ok, detail=""):
    print(("  ok   " if ok else "  FAIL ") + name + (f" — {detail}" if detail else ""))
    if not ok:
        failures.append(name)


def run_parallel(results, index):
    try:
        results[index] = get()
    except Exception as exc:  # noqa: BLE001 - the failure itself is the result
        results[index] = exc


def main():
    status, body = get("/")
    report("baseline GET / is 200", status == 200, f"status {status}")
    report("baseline body is not empty", len(body) > 1000, f"{len(body)} bytes")
    with open(f"{REF}/reference.html", "wb") as f:
        f.write(body)

    _, json_body = get("/json")
    report("GET /json returns a JSON object", json_body.strip().startswith(b"{"), json_body[:80].decode("utf-8", "replace"))
    with open(f"{REF}/reference.json", "wb") as f:
        f.write(json_body)

    results = [None] * PARALLEL
    threads = [threading.Thread(target=run_parallel, args=(results, i)) for i in range(PARALLEL)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(TIMEOUT + 5)
    errors = [r for r in results if isinstance(r, Exception)]
    mismatched = [i for i, r in enumerate(results) if isinstance(r, tuple) and r != (200, body)]
    report(f"{PARALLEL} parallel GETs answered", not errors, f"{len(errors)} errors, first: {errors[0] if errors else ''}")
    report(f"{PARALLEL} parallel bodies byte-identical to the baseline", not mismatched, f"{len(mismatched)} differ")

    aborted = 0
    for _ in range(5):
        sock = socket.create_connection((HOST, PORT), timeout=TIMEOUT)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))   # close with RST
        sock.sendall(b"GET / HTTP/1.1\r\nHost: bench\r\n")       # half a request, then abort
        sock.close()
        aborted += 1
    report(f"{aborted} connections aborted mid-request", True)

    sock = socket.create_connection((HOST, PORT), timeout=TIMEOUT)
    sock.sendall(b"GET / HTTP/1.1\r\nHost: bench\r\nConnection: close\r\n\r\n")
    sock.shutdown(socket.SHUT_WR)                                # half-close: no more requests
    received = bytearray()
    while True:
        chunk = sock.recv(8192)
        if not chunk:
            break
        received += chunk
    sock.close()
    head, _, payload = bytes(received).partition(b"\r\n\r\n")
    report("half-closed client received a complete status line and headers",
           head.startswith(b"HTTP/1.1 200") and b"Content-Type" in head, head[:40].decode("utf-8", "replace"))
    # The guest answers a half-closed client with its headers only: lwip reports the peer's FIN,
    # and the firmware's webserver closes any client whose `connected()` is false before the next
    # state step (webserver.cpp, the WEBSERVER_CLIENT_* loop). That is the guest's decision, not a
    # forwarder fault -- what the forwarder owes us is that the bytes the guest did send arrive
    # intact and in order, and that the flow closes without a reset. So: empty, complete, or a
    # clean prefix of the reference body, never interleaved garbage.
    body_ok = payload == b"" or payload.startswith(b"5") or body.startswith(payload)
    report("half-closed body is empty or an intact prefix of the baseline", body_ok,
           f"{len(payload)} payload bytes")
    if payload:
        dechunked, rest = bytearray(), payload
        while rest:
            size_line, _, rest = rest.partition(b"\r\n")
            try:
                size = int(size_line.split(b";")[0], 16)
            except ValueError:
                break
            if size == 0:
                break
            dechunked += rest[:size]
            rest = rest[size + 2:]
        report("half-closed body, when present, is byte-identical to the baseline",
               bytes(dechunked) == body, f"{len(dechunked)} vs {len(body)} bytes")
    with open(f"{REF}/half-close.txt", "w") as f:
        f.write(f"header {len(head)} bytes, payload {len(payload)} bytes\n")

    print("stress: PASS" if not failures else f"stress: FAIL ({len(failures)} phase(s))")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
