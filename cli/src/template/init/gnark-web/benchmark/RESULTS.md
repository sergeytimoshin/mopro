# Browser proving measurements

Measured September 10, 2026 on an Apple M5 Pro (18 CPUs), macOS, Go 1.26.3
and Brave Headless Chromium 152. Experimental browser arithmetic used 18 threads on a
cross-origin isolated page. Native gnark used the same generated keys and
witness, with its default 18-CPU configuration. Builds used gnark 0.14.0,
gnark-crypto 0.19.0 and the release WASM accelerator with batched affine MSMs,
affine weighted bucket sums, cached FFT plans, fused base-field products and
preallocated scalar-digit buffers. Larger windows split into 256-bucket jobs.
G2 affine additions batch denominator norms
in the base field. Hint-free R1CS circuits use a prepared Rust
solver that shares repeated linear-expression sums.

| Fixture command arguments | Constraints | Browser warm proof | Native warm proof | Ratio |
| --- | ---: | ---: | ---: | ---: |
| `-rounds 16384` | 16,385 | 66.1 ms | 37.5 ms | 1.8× |
| `-mimc -rounds 64` | 21,121 | 67.1 ms | 48.0 ms | 1.4× |
| `-rounds 2048 -commitments 2 -commit-all` | 2,053 | 19.7 ms | 30.4 ms | 0.65× |

Browser values are medians of 31 complete proofs. Native values are the mean
reported by `go test -bench BenchmarkWebFixture -benchtime=31x`. Each measurement
includes witness construction, solving, blinding and serialization. It excludes
key preparation and verification. The browser loaded files extracted from the
generated npm tarball. Every browser proof, including proofs for
changed and zero-valued inputs, was verified by native gnark. Timings vary with
hardware, browser scheduling, circuit structure and thread count. The commitment
fixture's ratio does not imply native speed for ordinary circuits.

With `MOPRO_GNARK_THREADS=0`, the same square fixture took 1,436.0 ms per warm
browser proof. The threaded implementation was about 22× faster on this fixture.

Ordinary circuits still take about 1.4–1.8 times as long as native
gnark, with most remaining time spent in the FFT and curve arithmetic.

Compressed-key preparation took 5.27 seconds for the square fixture, 4.78 seconds
for MiMC and 0.69 seconds for the commitment fixture. In an earlier run,
transcoding the same MiMC
keys using `WriteRawTo` reduced preparation to 2.25 seconds, with proving-key
size increasing from 3,794,847 to 7,547,231 bytes. Both formats use safe key
validation. Download time is excluded; raw keys leave the prepared arithmetic
unchanged. Startup took 73–215 ms across the three runs in one browser session; earlier
fresh browser sessions took 180–206 ms.

Run the fixture generator in `gnark-web/`, then `benchmark/browser.cjs` from
`web/` with `MOPRO_GNARK_MODE=rust MOPRO_GNARK_THREADS=18`. Use `BenchmarkWebFixture` and
`TestBrowserBenchmarkProofs` for the native comparison and independent proof
checks. Preserve each fixture/report pair before generating new keys.

These measurements describe the experimental engine. Default browser proving uses Go.
