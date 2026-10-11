# EX225: classic ESP32 crypto adapters

Base: branch `esp32-classic-3-peripherals-ff` (#207),
`schematik-engineering/esp32sim@8457808ee973a3975139707a1a542f28214dbcc1`, on upstream main `ff64902a`. Builds on EX230/EX224.
Adds classic AES and RSA registers and expands the boot SHA-256 adapter to
SHA-1/256/384/512, sharing AES, SHA and big-integer arithmetic with other chips.
The classic RSA Montgomery operation uses the programmed M-prime value;
plain multiply and modular exponentiation use the shared Rsa register operations.
The adapter translates operand widths and retains only classic commands and
Montgomery reduction. SHA counts come from the three shared engines, with
SHA-384/512 sharing a counter as well as state. DPORT owns command gating;
reset replaces each device with its initial state, including diagnostic counts.
DPORT clocks, coupled resets, RSA power-down and source 51 route completion.

## Rebased validation

Rust 1.99.0 on the ff64902a-based source: both Clippy targets pass with warnings
denied; 788 CI-mode workspace tests and 766 plain tests
(36 ignored) pass with empty HOME. All eight production WASM scenarios
and evidence privacy pass. Classic and existing goldens remain byte-identical.

## Sources and limits

ESP-IDF v5.5.4 `components/soc/esp32/include/soc/hwcrypto_reg.h:12-64`
defines operand memories and commands. In `register/soc/dport_reg.h`,
lines 98-104 define clock/reset bits, 1632-1638 and 2184-2190 define PRO/APP
RSA interrupt routing. Completion is synchronous and inferred. No timing,
hardware or IDF 4.4 validation is claimed. No new fields precede existing
peripheral fields; no crypto processing is added to the tick path.

The inherited IDF 5.5.4 fixture, ROM sources and input hashes are recorded in
[EX230](../classic-core/README.md). Tests use FIPS-197 AES vectors, SHA vectors
and independent integer arithmetic, including carry across every supported
Montgomery width. SHA-384/512 share state; SHA-1/256 remain independent.
AES key/text/output register layouts are also checked against literal register
words from ESP32 TRM v5.8 tables 14.3-2 and 14.3-3 (FIPS-197 C.1 vector) for
all 64 endian settings, independent of the test's write helper.

Only AES/SHA/RSA start and load commands are gated while the DPORT clock is
off; key, text, endian and operand-memory writes are stored regardless. Dropping
those writes while the clock is gated is not modeled (inferred; IDF enables the
clock before writing them).

## Verification

Run the full [EX230 command set](../classic-core/README.md) with Rust 1.99.0:
both Clippy targets with warnings denied, empty-HOME CI-mode and plain release
workspace tests, production WASM build and all eight required scenarios,
and evidence privacy. Existing goldens are not regenerated. JIT code is unchanged.

[Mutation table](mutations.json): 22 single replacements killed by assertion
failures. For each row, apply the replacement, run
`cargo +1.99.0 test -p esp32 --lib TEST`, then restore the source.

Results: both Clippy checks pass; 758 CI-mode tests and 736 plain tests
(36 ignored) pass with empty HOME. All eight WASM scenarios pass.
Existing goldens remain byte-identical. Evidence privacy passes.

## CPU comparison

Not measured separately. The stack tip, PR #212 (schematik-engineering/esp32sim@bcd4df7e, branch `esp32-classic-host`, which contains this change), was measured against main `2f9443a9` with the method in the classic-host receipt (user CPU, one warmup and seven alternating load-gated pairs, identical instruction counts and console hashes):

| Workload | Main median (range) | PR median (range) | Change | PR slower in N/7 | Instructions | max load |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| C6 hello | 3.2784 (3.0442–3.3351) | 3.2612 (3.0412–3.3331) | -0.52% | 2/7 | 4800000000 | 5.27 |
| Pocket Tank | 35.2968 (34.6376–38.3461) | 35.0204 (34.7165–38.7101) | -0.78% | 3/7 | 10073833665 | 4.26 |
| S3 hello | 24.6510 (23.7365–32.0593) | 24.3939 (23.8923–27.6721) | -1.04% | 4/7 | 1789819657 | 4.96 |
| C3 hello | 2.3837 (2.3635–2.5350) | 2.4178 (2.3742–2.5332) | +1.43% | 5/7 | 4800000000 | 3.53 |

That measurement predates the Oct 11 review commits and the rebase onto main `eec7b2c9`. Those commits add no S3/C3/C6 execution-path instructions; the static check below was repeated on the rebased stack tip (#211).

Static check (no timing): release `esp32sim` binaries from main `eec7b2c9` and
the rebased stack tip were disassembled with `llvm-objdump -d` and compared function by
function after normalising addresses, alignment `nop`s, adrp page offsets and
linker-chosen symbol aliases. The S3/C3/C6 execution path is instruction-identical
to main: `Machine::run`, `step_core`, each `SocBus` `tick` and `next_deadline`, S3 `tick_impl`,
`refresh_tick_budget` and GDMA engines, the MMIO dispatchers, `TimerGroup::tick`, and the UART,
TIMG, SHA, LEDC and RMT tick and register paths, and RTC_CNTL. Remaining differences: shared `Gpio::write` (`ubfx` for `lsr`, inherited from EX230); `I2c::write`, whose bounds-check panic calls share one source location in the macro
expansion, so three merge and the END arm moves; `Ledc::output`, the host PWM query, and the `Peripherals::new` constructors.

## Privacy

Only public source expressions, test names and outcomes are retained.
No private captures or machine identifiers are included.
