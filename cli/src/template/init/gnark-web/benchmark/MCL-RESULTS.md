# mcl MSM browser comparison

The experimental mcl backend is slower and larger than the Arkworks backend
in this configuration. Keep Arkworks as the default. This branch demonstrates
Rust-to-C FFI inside threaded WASM; it does not establish a reason to replace
the existing accelerator.

## Complete proof time

Both variants use Arkworks 0.5 for FFT and the same Go WASM for solving,
proof assembly and verification. Only MSMs and their input/output adapters differ.
The mcl variant uses upstream G1/G2 MSMs, including commitment MSMs, with
canonical scalar conversion and prepared point buffers. Rayon schedules chunks
of at least 1,024 bases on the existing worker pool.

| Circuit | Constraints | Arkworks MSM | mcl MSM | mcl slowdown |
| --- | ---: | ---: | ---: | ---: |
| 16,384 squarings | 16,385 | 220.1 ms | 438.1 ms | 1.99× |
| MiMC, 64 inputs | 21,121 | 326.6 ms | 545.1 ms | 1.67× |
| 2,048 squarings, 2 commitments | 2,053 | 60.7 ms | 247.7 ms | 4.08× |

Measured September 11, 2026 on Linux, AMD Ryzen 9 9950X (16 cores / 32 logical
CPUs), Chrome for Testing 153.0.8010.12, 16 browser workers, Go 1.24.0,
nightly-2025-11-15 and wasm-pack 0.15.0. mcl was compiled with Clang/LLVM 18.1.3.
Shared Rust dependencies, release settings, keys, circuits and witnesses match.

Each fixture/variant ran three separate browser sessions, each with five
warmups and 31 timed proofs. Variant order alternated between sessions. The
table reports the median of session medians. Verification runs outside the
timed interval. All 594 recorded proofs were independently verified by native
gnark. Each session also exercises an unsatisfied witness, changed and zero
inputs, expected backend selection, and disposal.

The harness records the compiled MSM backend and the delivered WASM SHA-256,
checked against each built artifact. These checks run after timing. The full
samples, fixture hashes, artifact hashes and session medians are retained in
[mcl-comparison.json](mcl-comparison.json).

## Size and preparation

| Artifact | Arkworks MSM | mcl MSM |
| --- | ---: | ---: |
| Accelerator WASM | 385,582 bytes | 649,738 bytes |
| Accelerator WASM, gzip | 116,730 bytes | 206,475 bytes |

The mcl accelerator is 68.5% larger uncompressed
and 76.9% larger with gzip. The identical
Go WASM is 19,804,119 bytes in both builds.
Accelerator Rust source, including both backends and differential tests, grows
from 342 to 592 lines. Build helpers and the pinned C++ dependency are additional.
This experiment does not shrink the maintained integration.

| Circuit | Arkworks preparation | mcl preparation |
| --- | ---: | ---: |
| square | 14.60 s | 14.67 s |
| mimc | 13.39 s | 13.38 s |
| commitments | 1.91 s | 1.92 s |

Preparation includes Go key validation and accelerator key conversion. Keys
are fetched before timing. Runtime startup is recorded separately; localhost
loading does not measure deployment network costs.

## Integration findings

`mcl-rust` is pinned to `81981d036f736ab96800987e1678d7ca15182f76`; its mcl
submodule is `a8cad811f720264e6e140f54c66463be60189789`. No arithmetic kernels
are copied into the accelerator. The Rust binding currently requires two
integration accommodations:

- The C MSM API may normalize point buffers in place. The adapter binds these
  calls with mutable pointers and checks dimensions instead of using the
  upstream shared-slice `mul_vec` wrappers.
- This linked WASM build needs explicit C++ constructor initialization before
  curve setup. Without it, native checks pass but a browser G2 MSM involving
  negative scalars produces the wrong result. Initialization now runs once;
  a dedicated browser differential test covers this failure.

The build also needs C++ shared-memory flags and LLVM archive tools. These
costs accompany the dependency reuse. The experiment remains opt-in through
`mcl-msm`; ordinary builds continue to use Arkworks.

Results apply to this implementation and desktop Chromium configuration.
They include conversion, allocation and scheduling costs. Worker chunk sizes
have not been exhaustively tuned; these measurements do not establish that
all mcl WASM integrations are slower. Safari, Firefox and mobile hardware
were not tested.

## Reproduce

Start with generated gnark applications containing the same accelerated web
bindings. Generate the square, MiMC and commitment fixtures once using the
commands in [RESULTS.md](RESULTS.md#reproduce), then share those exact files
between both web directories. Copy this branch's accelerator template into
the generated accelerator, renaming `Cargo.toml.template` to `Cargo.toml`.

Build each variant into its corresponding web bindings directory:

```sh
MCL_CLANGXX=clang++-18 MCL_LLVM_AR=llvm-ar-18 \
  bash gnark-web/accelerator/build-msm.sh mcl /absolute/mcl/web/MoproWasmBindings/gnark/accelerator
bash gnark-web/accelerator/build-msm.sh arkworks /absolute/arkworks/web/MoproWasmBindings/gnark/accelerator
```

Serve the directories with cross-origin isolation headers. Run the harness
from each generated web directory, using the appropriate URL and MSM name:

```sh
MOPRO_GNARK_MODE=rust MOPRO_GNARK_THREADS=16 \
MOPRO_GNARK_EXPECT_MSM=mcl MOPRO_GNARK_EXPECT_SOLVER=go \
MOPRO_GNARK_BENCH_SAMPLES=31 MOPRO_GNARK_BENCH_WARMUPS=5 \
MOPRO_GNARK_BENCH_URL=http://127.0.0.1:3000/gnark-benchmark.html \
MOPRO_GNARK_BENCH_BASE=./assets/square/ \
MOPRO_GNARK_BENCH_REPORT=/absolute/reports/square-mcl-1.json \
node /path/to/this/branch/cli/src/template/init/gnark-web/benchmark/browser.cjs
```

Repeat for three sessions and all three fixtures, alternating variant order.
Independently verify each report from a generated `gnark-web` directory:

```sh
MOPRO_GNARK_BENCH_FIXTURES=/absolute/fixtures/square \
MOPRO_GNARK_BENCH_REPORT=/absolute/reports/square-mcl-1.json \
go test -run '^TestBrowserBenchmarkProofs$' -count=1
```

Build `check` into `web/msm-check`, then run `benchmark/check-msm.cjs` from the
served web directory for the separate one- and sixteen-worker arithmetic checks.
The npm tarball and production Vite bundle also passed startup failure recovery,
with 18 additional bundled proofs independently verified by native gnark.
Native tests and Clippy passed for both feature selections; browser arithmetic
checks passed with one and sixteen workers.

Diagnostic code is excluded from the timed artifacts. Full proof reports,
verification logs, runtime artifacts and fixture files are retained in the
local `mopro-gnark-mcl-benchmark-2026-09-11.tar.gz` archive.
