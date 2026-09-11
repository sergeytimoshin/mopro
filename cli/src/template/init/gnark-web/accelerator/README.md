# Experimental gnark browser arithmetic

This internal WASM module computes the quotient polynomial, the five main
Groth16 MSMs and commitment MSMs using published Arkworks 0.5 crates.
Go retains witness solving, safe key decoding, the commitment protocol,
random blinding, proof serialization and verification.

`ark-bn254` / `ark-ff` provide field and curve arithmetic, `ark-ec` provides MSMs,
and `ark-poly` provides FFTs. Rayon and wasm-bindgen-rayon run the arithmetic
on browser workers. The adapter imports gnark's domain generator and coset,
applies the quotient denominator and permutes coefficients into the bit-reversed
order of gnark's Z key. It contains no custom field arithmetic, MSM or FFT kernel.

The wire format uses the same four little-endian Montgomery limbs as
gnark-crypto 0.19 and Arkworks 0.5. Go validates keys before constructing the
internal `Key`; direct JavaScript construction is not a supported API.

The build includes this engine only when the application's root `Cargo.toml`
sets `package.metadata.mopro.gnark.experimental-accelerator = true`.
`initGnark({ experimental: true })` starts it on cross-origin isolated pages.
Default proving, small circuits and pages without shared memory use Go.

Build through `mopro build --platforms web`. Run the adapter test with
`cargo test --manifest-path gnark-web/accelerator/Cargo.toml --workspace --release --locked`.
Browser tests exercise ordinary circuits, hints and commitments, assert the
selected backend, and independently verify every recorded proof with native gnark.
See `../README.md` for the component upgrade boundary and `../benchmark/RESULTS.md`
for measurements. Keep `CARGO_TARGET_DIR` outside the CLI template when building
from a source checkout.

## mcl MSM experiment

The optional `mcl-msm` feature replaces G1/G2 and commitment MSMs with mcl.
Arkworks FFT, the quotient adapter, Go solving and proof assembly stay in place.
Arkworks remains the default. The experiment pins `mcl-rust` at
`81981d036f736ab96800987e1678d7ca15182f76`, including its mcl submodule at
`a8cad811f720264e6e140f54c66463be60189789`. No arithmetic source is vendored.

mcl's `SNARK`/`BN_SNARK1` curve matches gnark BN254. Canonical bytes convert
between libraries; the existing gnark/Arkworks Montgomery wire format stays
internal to the Go bridge. Prepared keys retain mcl points. Rust gives mcl
exclusive point slices because its C MSM API may normalize them in place;
the upstream Rust wrapper's shared-slice `mul_vec` functions are not used.
The existing Rayon pool schedules complete upstream MSMs in disjoint chunks.
C++ constructors and global curve initialization run once before arithmetic.

For a generated application, build the standard accelerated bindings first.
Then select a comparison variant, passing an absolute output directory:

```sh
bash gnark-web/accelerator/build-msm.sh mcl "$PWD/MoproWasmBindings/gnark/accelerator"
bash gnark-web/accelerator/build-msm.sh arkworks "$PWD/MoproWasmBindings/gnark/accelerator"
```

mcl needs Clang and `llvm-ar`; select versioned executables with
`MCL_CLANGXX=clang++-18 MCL_LLVM_AR=llvm-ar-18`. `MOPRO_WASM_PACK` can select a
wasm-pack executable. The helper uses the same Rust toolchain and release flags
for both variants, enables shared-memory features in C++, and copies licenses.
It works with non-executable files extracted from CLI templates.

Native differential checks:

```sh
cargo test --manifest-path gnark-web/accelerator/Cargo.toml --release --locked --features mcl-msm
```

Build the `check` variant into `web/msm-check` and serve the web directory with
isolation headers. Run `benchmark/check-msm.cjs` from that web directory to
compare G1/G2 results with Arkworks using one and sixteen browser workers.
The diagnostic code is excluded from ordinary builds. It covers repeated keys,
identity bases, zero/full-width/negative scalars and parallel chunk boundaries.
See [the comparison](../benchmark/MCL-RESULTS.md) for timings and limitations.
