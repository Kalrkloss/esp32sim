**CPU table pending.** Our timing run (user CPU, 1 warmup + 7 alternating pairs against main, identical instruction counts and console hashes) needs a quiet machine and hasn't run yet. The table will be added to the receipt and posted here as soon as it is measured.

# Read-only NAT TCP occupancy

`Nat::tcp_flow_counts(&self) -> TcpFlowCounts` returns current transport occupancy:

- `active`: `Connecting` and `Connected`, including guest half-close, queued writes,
  a host FIN awaiting acknowledgement, and a completed exchange before `poll` moves it
  to `TimeWait`.
- `time_wait`: completed flows retained to re-ACK a retransmitted guest FIN.
- `Closed`: excluded immediately, even if the vector still retains the entry until `poll`.

The snapshot does not poll host sockets or connector channels, expire flows, refresh
activity, retransmit packets, or change cumulative statistics. A connector result that
is ready but not yet polled still counts as active. Idle/TimeWait expiry becomes visible
only after normal polling. This is an explicit O(n) query over the existing bounded TCP
table; there are no new fields, tick calls, policy rules, socket paths or admission
limits. UDP is unchanged. No existing CLI calls the query.

## Reproduction

Base: `f1b5c5d5ee218842de82af6ee5abda33429c97f7`.
Executable/test revision: `80d22688b73a0da0cc610d1f32fec1982b3e3ce2`.
Qualification on October 25, 2026 uses this revision and a documentation-only descendant
with identical executable/test blobs. Rust 1.99.0 (`b940084d7`), native arm64 macOS.

Use `nice -n 10`, `CARGO_BUILD_JOBS=4`, and a dedicated `CARGO_TARGET_DIR`. Fetch demo
assets with `tools/fetch-demo-assets.sh --no-linux`, `tools/fetch-p4-firmware.sh` and
`tools/fetch-micropython.sh` in the feature checkout, as CI does. For
`tools/wasm-build.sh`, create a checkout-local `target` symlink to the dedicated
external target directory (the script's copy step uses that path). The ten demo
manifests and every image they reference (`files`, per-node `files` and `flash_at`)
are hashed in `inputs.json`.

All commands below passed; commands and exit codes are retained in `checks.json`:

| Command (cargo commands use `nice -n 10 cargo +1.99.0`; mutations use `-j2`) | Result |
| --- | --- |
| `test --release -p esp-soc nat::tests` after restoring mutants | 22 passed, including eight counts tests |
| `clippy --workspace --all-targets -- -D warnings` | PASS |
| `clippy --release --target wasm32-unknown-unknown -p esp32sim-wasm --features jit-tests -- -D warnings` | PASS |
| Empty HOME, only `ESP32SIM_ROM_DIR` set: `test --release --workspace -- --include-ignored --skip external_` | 1,245 passed, zero failed/ignored |
| Empty HOME, no emulator settings: `test --release --workspace` | 1,195 passed, zero failed, 73 ignored |
| `RUSTUP_TOOLCHAIN=1.99.0 tools/wasm-build.sh` | PASS |
| `node tools/wasm-test.mjs hello c3-hello c6-hello c6-energy-scan c6-contiki c6-contiki-net c6-rpl-net panel` | All eight scenarios PASS |
| `ESP32SIM_ROM_DIR=web/wasm/fw node tools/wasm-test.mjs p4-hello tdisplay-prices` (the rest of CI's list) | Both scenarios PASS |
| `node tools/check-evidence-privacy.mjs` | PASS; pattern scan plus manual new-file review |
| `git diff --check` | PASS |

For the two empty-HOME runs, retain explicit `RUSTUP_HOME`/`CARGO_HOME` for toolchain
access, remove inherited emulator settings, and set `ESP32SIM_ROM_DIR` to the absolute
checkout-local `web/wasm/fw` path only for the CI-policy run. `test-totals.json` records
the workspace totals. No JIT source changes; the additional JIT-specific script is not
required. No tracked firmware or goldens change. `docs/experiments.md` is byte-identical
to the named main base; this API is not a speed, timing or execution experiment.

The deterministic host write-failure regression uses a locally shut-down socket to
exercise the host-I/O-error/RST path, rather than relying on platform-dependent peer
RST timing/linger. It is not a packet-level host-RST integration test.

Tests use only the existing private injected connector and loopback socket helpers:
empty/failed admission, pending/ready/refused/disconnected connector, guest reset,
mixed states and cumulative-statistics independence, host write failure, draining
and half-closed guest, unacknowledged host FIN, final-ACK retry, TimeWait timeout and
active idle timeout. Immutable/repeated snapshots check that pending connector
results and protocol bookkeeping remain untouched.

## Mutations

Run `python3 docs/evidence/nat-tcp-counts/mutate.py "$OUT"` with a dedicated
`CARGO_TARGET_DIR`. The script restores the original source in `finally` and reruns
the complete NAT unit-test module after its named mutants.

Nine mutants fail named runtime assertions: omit Connecting, omit Connected, count
Closed as active, classify TimeWait as active, omit TimeWait, drop guest half-close,
drop unacknowledged host FIN, consume a ready connector result, and use cumulative
statistics as occupancy. The tenth changes the receiver to `&mut self`; the immutable
borrow caller rejects that contract at compilation (E0596). It is not a runtime
assertion kill. Each runtime log was checked for an assertion panic, not merely a
nonzero exit code.

All ten named mutants were re-killed on the named base; exact test names and outcomes
are retained in `mutations.json`. The restored NAT suite passed all 22 tests, and
restored source/test blobs match the executable revision exactly.

## CPU comparison

PENDING

No timing claim. Separate exact-base/head native CLI release builds and normalized
function comparisons establish whether existing executable paths changed. See
`static.json` for the fresh results, binary hashes and disassembly hashes. Alignment,
linker addresses and equivalent folded symbol aliases are normalized, not arbitrary
instructions or field offsets. Since the query is unused by existing CLIs, this does
not measure caller-side query cost.

Each of the four CLIs has 2,608 normalized-identical functions, zero different
functions and zero missing/added functions against the exact base. This includes all
existing emulator hot paths and NAT traffic paths. Raw disassembly instruction text
also matches after removing only the filename/file-format heading and blank lines,
before instruction normalization. Binary bytes differ; this is not CPU-time clearance.

Build each checkout with `nice -n 10 cargo +1.99.0 build --release --bins`, using
separate target directories and four jobs. For each of `esp32sim`, `esp32sim-c3`,
`esp32sim-c6` and `esp32sim-p4`, disassemble with
`xcrun llvm-objdump --disassemble --demangle --no-show-raw-insn`, then run
`python3 docs/evidence/nat-tcp-counts/compare.py base.dis head.dis`.
This establishes instruction equality only where the comparison reports it, not
identical binary bytes or CPU-time clearance.

## Limits

This API exposes counts, not flow identities or layout. It does not itself fix any
consumer's admission or retention behavior. Integration, deployment, hardware,
remote CI and CPU qualification are outside this receipt. No firmware goldens are
intentionally changed. No private traffic or personal data are inputs. Evidence is
curated commands, hashes and aggregate outcomes; no private captures were collected.
