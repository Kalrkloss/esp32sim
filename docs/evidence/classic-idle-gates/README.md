# Classic idle adapter gates

Base: `ab89dc543bd8aa23b2d39f2cc48348ea5a0793ea` (upstream main after #212).
This is deterministic attribution, not a CPU percentage estimate or a timing benchmark.
The workload is the committed classic hello fixture, ECO3 ROM, esp32dev, UART0,
30 emulated seconds including two resets, no Wi-Fi or BLE configuration.
Input hashes and build/disassembly hashes are in `manifest.json`.

## Observed per-part attribution

Denominator: 9,639,235 calls of the classic bus tick, identical on base and candidate.
The temporary counters count adapter entries separately from entries into the actual
clocked-work body. Instrumentation is restored before normal builds and tests.
`base-attribution.json` and `candidate-attribution.json` retain all counters.

| Part | Base adapter calls | Candidate adapter calls | Clocked-work entries, both | Observed gates |
| --- | ---: | ---: | ---: | --- |
| Wi-Fi | 9,639,235 | 0 | 0 | Combined WIFI_CLK_EN / MAC-reset predicate enabled on 0 ticks |
| LEDC | 9,639,235 | 0 | 0 | Cached DPORT clock/reset predicate enabled on 0 ticks |
| RMT | 9,639,235 | 0 | 0 | Cached DPORT clock/reset predicate enabled on 0 ticks |
| I2S (two-port inline loop) | 9,639,235 | 9,639,235 | 0 port-work entries | I2S0 clock on for 94,288 ticks; I2S1 clock on for 0; each reset asserted for 4,717 ticks; no DMA running |
| ADC | 0 tick entries | 0 tick entries | 0 conversion calls | RTC/SENS access-driven, not a DPORT tick device |
| DAC | 0 tick entries | 0 tick entries | 0 output queries | RTC_IO power/mux access-driven, not a DPORT tick device |
| Touch | 0 tick entries | 0 tick entries | 0 measurement calls | RTC/SENS access-driven, not a DPORT tick device |
| VHCI glue | 0 intercept entries | 0 intercept entries | 0 | Opt-in controller glue; no bus tick hook or DPORT gate |

ADC/DAC/touch/VHCI entries are counted at their real access/intercept entry points,
not inferred by assigning every new adapter a bus tick. The ADC SENS device is an
alias in the dispatch table and has no clock() implementation.

## Explanation and scope

Wi-Fi and LEDC/RMT already returned internally when gated off. They were not doing
frame processing, timer advancement or channel advancement in this workload.
The regression mechanism here is repeated adapter entry/gate checking, not active
peripheral emulation with the clock stopped. Wi-Fi read the DPORT register map once
per idle tick; I2S read it for both ports even with no DMA pending. LEDC/RMT were
called after Dispatch on every tick regardless of their clock()/pending-work result.

The fix caches Wi-Fi and I2S gate predicates only on DPORT writes, leaves the existing
LEDC/RMT gate cache intact, and uses the adapters' clock() predicate for manual APB
delivery. The idle workload avoids 28,917,705 Wi-Fi/LEDC/RMT adapter calls. The inline
I2S loop remains, but checks cached booleans instead of reading the register map;
its work body requires both an enabled, non-reset port and a running DMA channel.
This also fixes the missing I2S reset check. Public bit definitions:
ESP-IDF v5.5.4 `components/soc/esp32/register/soc/dport_reg.h`, clock bits at
947/965, reset bits at 982/1000, Wi-Fi mask at 1040-1043 and MAC reset at 1064/1069.
LEDC/RMT output synchronization and completed-output delivery are retained even with
clocks off, so already queued GPIO/RMT changes are not lost.

This evidence cannot apportion the maintainer's +10.2% by percentage, or establish
that this patch recovers it. No wall/user-CPU comparison was made. DMA reach rules
are unchanged. No hardware equivalence beyond the existing model is claimed.

## Guest equivalence

Uninstrumented native classic hello at 30 seconds, both revisions:
core0 16,672,061 instructions; core1 4,515,255 instructions;
27,408 exceptions; 6,016 interrupts; console SHA-256
`13aadb96ac2d9e7d827e9daea68ec3f5dd70de46bb0c2eb60ee2eaad4f7539ec`.
The counter-instrumented runs give these exact same counts and hash.

`demo-comparison.json` records exact instruction counts and console hashes for the
entire standard WASM set: hello, c3-hello, c6-hello, c6-energy-scan, c6-contiki,
c6-contiki-net, c6-rpl-net and panel. All eight match main; the two network manifests
include each node's instruction count and console hash. Only the evidence build
adds a read-only net_stat index 7 to expose existing instruction counts; it is not
part of the patch or the shipped ABI. Binary frame counts and network TX/RX also match.
All native golden tests pass against unchanged committed goldens, with no UPDATE_GOLDENS.

## Mutation checks

`mutate.py` changes one gate, runs its named test, requires an assertion failure
(not a compiler failure), then restores the source. All eight mutations were killed.
The restored esp32 suite passes: 94 unit tests, 15 peripheral tests and 8 TLS tests.

| Mutation | Killing test |
| --- | --- |
| Remove LEDC call-site clock/pending predicate | classic_idle_gates_skip_adapter_calls |
| Remove RMT call-site clock/pending predicate | classic_idle_gates_skip_adapter_calls |
| Remove Wi-Fi bus call-site gate | classic_wifi_clock_and_reset_skip_step |
| Ignore Wi-Fi clock mask | classic_wifi_clock_and_reset_skip_step |
| Ignore Wi-Fi MAC reset | classic_wifi_clock_and_reset_skip_step |
| Ignore I2S port clock bit | classic_i2s_clock_reset_and_pending_gates |
| Ignore I2S port reset bit | classic_i2s_clock_reset_and_pending_gates |
| Ignore I2S pending-DMA predicate | classic_i2s_clock_reset_and_pending_gates |

Counters in adapter structs are cfg(test)-only: no production counter fields or increments.

## Checks

All use Rust 1.99.0 and nice -n 10 with CARGO_TARGET_DIR outside the control checkout.

- `cargo +1.99.0 clippy --workspace --all-targets -- -D warnings`: PASS.
- `cargo +1.99.0 clippy --release --target wasm32-unknown-unknown -p esp32sim-wasm --features jit-tests -- -D warnings`: PASS.
- Empty HOME, only ESP32SIM_ROM_DIR for fixture discovery, `cargo +1.99.0 test --release --workspace -- --include-ignored --skip external_`: 941 passed, 18 filtered, no failures.
- Empty HOME and no ESP32SIM_ROM_DIR, `cargo +1.99.0 test --release --workspace`: 913 passed, 46 ignored, no failures.
- `RUSTUP_TOOLCHAIN=1.99.0 tools/wasm-build.sh` and `node tools/wasm-test.mjs hello c3-hello c6-hello c6-energy-scan c6-contiki c6-contiki-net c6-rpl-net panel`: PASS, including JIT handoff.
- `node tools/check-evidence-privacy.mjs`: PASS; curated evidence manually checked too.
- `git diff --check`: PASS. No JIT source change; separate JIT differential suite not required.

The WASM build script assumes ./target for its copy; ./target is a symlink to the
external CARGO_TARGET_DIR. No build occurs in the control checkout.

## Hot-path disassembly

Build both revisions with `cargo +1.99.0 build --release --bins`, then
`llvm-objdump --disassemble --demangle --no-show-raw-insn` on each esp32sim binary.
Use the committed classic-core asmcmp.py normalization through `disasm.py`.
`classic-hot.json` lists 27 selected functions: 21 identical, six changed.
The 119-function S3/C3/C6/shared selection from classic-core/hot.py is entirely
identical (119 same, 0 different, 0 missing).

Classic bus tick is 1,854 static instructions on both revisions, but not identical:
manual LEDC/RMT calls are now conditional on clock()/pending work, and the inlined
Wi-Fi/I2S paths use cached booleans instead of DPORT map lookups. Register allocation
changes around those predicates. Static function size does not measure the executed
idle path; the call counters establish which engines are actually entered.
The other five changed functions (after_round, run_modeled, run_unmodeled,
settle_modeled_time and step_core) preserve every opcode/register/order and differ
only by +16-byte field offsets after the enlarged two-port I2S adapter array.
The cached flags are added to adapter tails, not ahead of core fields; Rust's layout
still enlarges the enclosing peripheral block. LEDC/RMT engine and output-sync
function bodies remain identical. Full raw disassemblies/diffs are retained outside
Git; their hashes are in manifest.json, and the scripts reproduce the comparison.

## Reproduction

Set EVIDENCE_OUT to a writable external evidence directory. Scripts are sequential:
they temporarily instrument/restore this worktree and must not overlap builds here.
Run attribute.py WORKTREE base, then attribute.py WORKTREE candidate; run mutate.py
WORKTREE and compare_demos.py WORKTREE. compare_native.py expects uninstrumented
binaries under EVIDENCE_OUT/target-base/release and EVIDENCE_OUT/target/release.
disasm.py expects base.dis and candidate.dis under EVIDENCE_OUT. Fetch assets with
`tools/fetch-demo-assets.sh --no-linux` in each worktree before running firmware.

## CPU comparison

PENDING — measured centrally later, without concurrent builds.
