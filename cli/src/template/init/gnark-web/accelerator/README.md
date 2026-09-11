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
