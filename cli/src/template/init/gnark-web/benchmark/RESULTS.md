# Browser accelerator comparison

Measured September 11, 2026 on Linux, AMD Ryzen 9 9950X (16 cores / 32 logical
CPUs), Chrome for Testing 153.0.8010.12 and Go 1.24.0. Both Rust engines used
16 workers, nightly-2025-11-15, wasm-pack 0.15.0 and identical release flags.
The original is commit `2bb184c23463d6415ebb5c9110bfd779df910d2f`; the smaller
engine uses published Arkworks 0.5 crates and keeps all witness solving in Go.
The versions of shared Rust dependencies are unchanged.

Each variant ran three separate browser sessions per circuit. Every session
used five warmups and 31 timed complete proofs, with proof verification outside
the timed interval. The table reports the median of the three session medians.
The run order rotated between sessions to distribute order effects. Go-only
measurements use the smaller branch's ordinary Go prover with no Rust workers.
Key preparation and startup are measured separately. Keys are fetched before
timing; startup includes loading runtime assets from localhost. These runs do
not measure network transfer performance.
All variants use the same freshly generated compressed keys and witnesses.

These results compare the complete implementations. They do not isolate the
cost of removing field multiplication, MSM, FFT or solver optimizations.
The earlier Apple M5 Pro / Go 1.26 measurements remain available at the original
commit and should not be compared directly with this machine.

## Warm proving

| Circuit | Constraints | Original | Published crates | Go only | Slower than original | Faster than Go |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 16,384 squarings | 16,385 | 73.6 ms | 218.0 ms | 3766.0 ms | 2.96× | 17.27× |
| MiMC, 64 inputs | 21,121 | 79.4 ms | 328.0 ms | 3913.3 ms | 4.13× | 11.93× |
| 2,048 squarings, 2 commitments | 2,053 | 22.2 ms | 59.6 ms | 1015.6 ms | 2.68× | 17.03× |

All 891 recorded benchmark proofs were independently verified with native gnark.
Every browser session also checks an unsatisfied witness, changed and zero-valued
inputs, backend selection and disposal. The comparison is limited to this
single desktop/browser configuration. The production Vite bundle, worker startup
recovery, built-in hints and portable fallback checks also passed, with another
40 browser proofs independently verified natively.

Commitment circuits use Go witness solving in both engines, so their slowdown
reflects changes to the arithmetic path. Separating the effects of field
multiplication, MSM, FFT and solver changes requires further measurements.

## Source and artifact size

| Measure | Original | Published crates |
| --- | ---: | ---: |
| Accelerator Rust source lines, including tests and vendored code | 3,152 | 342 |
| Rust WASM bytes | 496,632 | 385,214 |
| Rust WASM gzip bytes | 143,401 | 116,997 |
| Go WASM bytes | 19,814,867 | 19,804,119 |

The smaller engine removes 89.1% of accelerator Rust source and reduces the Rust
WASM by 22.4% (18.4% with gzip). The Go runtime dominates the overall package size.
The remaining Rust source implements the internal key/field bridge, quotient
normalization and key ordering, plus an adapter test. The Go solver export and
its tests are also removed.

## Preparation

| Circuit | Original preparation | Published-crate preparation |
| --- | ---: | ---: |
| square | 14.78 s | 14.49 s |
| mimc | 13.67 s | 13.47 s |
| commitments | 1.93 s | 1.90 s |

Both engines still validate the same compressed keys in Go. Startup
ranged from 267–280 ms for accelerated sessions and 240–256 ms for Go-only
sessions; preparation remains the larger initial cost for these fixtures.

## Reproduce

Build generated gnark applications from the original commit and this branch.
Patch each app's `mopro-ffi` dependency to its corresponding checkout and set
`package.metadata.mopro.gnark.experimental-accelerator = true`. Keep the same
Go compiler, Rust toolchain and accelerator release settings for both builds.
Use `benchmark/check-package.cjs` with `MOPRO_GNARK_ACCELERATOR=true` to install
both npm tarballs into their generated `web/MoproWasmBindings` directories.

Generate the fixtures once in either app's `gnark-web/` directory, then copy
those exact directories into both apps' `web/assets/` directories:

```sh
go run ./cmd/benchmark -rounds 16384 -out ../web/assets/square
go run ./cmd/benchmark -mimc -rounds 64 -out ../web/assets/mimc
go run ./cmd/benchmark -rounds 2048 -commitments 2 -commit-all -out ../web/assets/commitments
```

Serve both web directories with cross-origin isolation headers. From each
`web/` directory, use this branch's `benchmark/browser.cjs` for both versions:

```sh
MOPRO_GNARK_MODE=rust MOPRO_GNARK_THREADS=16 \
MOPRO_GNARK_BENCH_SAMPLES=31 MOPRO_GNARK_BENCH_WARMUPS=5 \
MOPRO_GNARK_EXPECT_SOLVER=go \
MOPRO_GNARK_BENCH_BASE=./assets/square/ \
MOPRO_GNARK_BENCH_REPORT=square-minimal-1.json \
node /path/to/this/branch/cli/src/template/init/gnark-web/benchmark/browser.cjs
```

Set `MOPRO_GNARK_BENCH_URL` when using a server URL other than
`http://localhost:3000/gnark-benchmark.html`. For the original engine, set
`MOPRO_GNARK_EXPECT_SOLVER=rust` for square and MiMC; commitments use `go`.
For Go-only proving, use `MOPRO_GNARK_MODE=go` and expect the Go solver.
Repeat for all fixtures and three sessions, rotating the variant order as
recorded in `comparison.json`.

For every saved report, independently verify its proofs from `gnark-web/`:

```sh
MOPRO_GNARK_BENCH_FIXTURES=../web/assets/square \
MOPRO_GNARK_BENCH_REPORT=../web/square-minimal-1.json \
go test -run '^TestBrowserBenchmarkProofs$' -count=1
```

`comparison.json` retains all timings, execution assertions, compiler information
and hashes of the shared fixtures and built WASM files. It omits proof bodies;
the benchmark harness writes those to its full reports for native verification.
