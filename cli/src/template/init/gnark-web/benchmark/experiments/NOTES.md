# Integration scope

The deployed Arkworks branch is unchanged. These branches compare backend
implementations behind the existing gnark arithmetic boundary.

- `feat/gnark-web-browser-bench`: shared asynchronous bridge, phase diagnostics,
  fixture conversion, arithmetic checks and reproducible benchmark harness.
- `feat/gnark-web-montgomery`: pinned montgomery 0.4.0 for G1. Arkworks retains G2
  and FFT. The Rust module is dynamically imported and its namespace retained so
  Vite preserves Rayon's worker entry point. Both libraries load only after the
  isolation guard, preserving Go fallback on ordinary pages. The `hybrid-only`
  build omits G1 MSM.
- `feat/gnark-web-ffjavascript`: pinned ffjavascript 0.3.1 / wasmcurves 0.2.2 for
  G1, G2 and FFT. Its QAP join operation is reused. No Rust arithmetic module is
  included in the tested package.
- `feat/gnark-web-arkworks-tuning`: opt-in `ark-glv` and `serial-msm` features.
  GLV reuses upstream scalar decomposition, endomorphism and variable-base MSM.

## Limitations relevant to adoption

The hybrid uses separate Rayon and montgomery pools. At the 16-thread setting it
creates 16 Rayon workers and 15 montgomery workers plus the prover coordinator.
Its main G1 MSMs run sequentially because the library's shared scratch allocator
and worker barriers are not safe for concurrent independent MSM calls. G2 and
FFT currently also run before those MSMs. Arkworks runs its five main MSMs
concurrently. Per-MSM elapsed times under concurrency cannot be added or compared
to isolated operations as if they were CPU times.

montgomery does not expose an independent allocation/free API for prepared keys.
The adapter reuses blocks and checks the pinned MSM result-allocation contract
before rewinding every pool member. This is additional integration coupling. It
uses safe additions, handles infinity explicitly, and tests multiple live keys.
It does not use `msmUnsafe` or benchmark-only random bases in actual proof keys.

ffjavascript chooses its worker count from navigator.hardwareConcurrency. The
experiment temporarily overrides that property inside the prover worker while
constructing the pool, restores it, and asserts the resulting pool size. The
outer browser's hardware count is never changed. A public pool-size API would be
preferable for an upstream integration. ffjavascript/wasmcurves/wasmbuilder use
GPL-3; that dependency choice requires separate consideration before adoption.

The experiment preserves the existing public `execution.arithmetic` accelerated
path label (`rust`) for compatibility with the shared prover tests. The separate
`backend` field identifies and asserts the actual library, including ffjavascript
which performs all accelerated arithmetic in its own generated WASM. This is
benchmark metadata, not a proposed final public API for additional backends.

The GLV experiment doubles the MSM basis and passes decomposed scalars to the
existing upstream MSM. That MSM still uses the scalar field's full bit width;
this is not a custom implementation optimized around 128-bit GLV components.
Decomposition also uses upstream arbitrary-precision integer allocation. Timing
this adapter does not establish that every GLV implementation is slow.

Preparation measurements exclude key download. Raw keys avoid decompression but
retain gnark's normal curve/subgroup and key-dimension checks. The report includes
raw and gzip byte counts to show the transfer tradeoff. Browser timings cover
only desktop Chromium on the documented machine, not mobile Safari/Firefox.
