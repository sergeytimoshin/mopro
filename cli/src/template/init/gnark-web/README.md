# Gnark browser runtime

This module runs gnark's Go prover and verifier in a dedicated Web Worker.
It supports Groth16 over BN254 with gnark 0.14 and gnark-crypto 0.19, matching
`rust-gnark` 0.0.2 serialization. The native C bridge is not used in the browser.

Build with `mopro build --platforms web` or the generated `cargo run --bin web`
helper. Both package the Go WASM, its matching `wasm_exec.js`, module worker,
JavaScript API and TypeScript declarations in `MoproWasmBindings/gnark/`.
Keep these assets together when deploying. The top-level Mopro WASM build still
requires its normal Rust/WASM toolchain.

Serve over HTTP(S) with module workers, WebAssembly and Web Crypto available.
Cross-origin isolation and shared memory are not required for gnark. Chromium
is covered by CI; Firefox, WebKit and iOS are not yet validated by this branch.

Use `prepareGnarkCircuit(r1cs, { provingKey, verifyingKey })` to retain decoded
circuit/key data across proofs. Witnesses are JSON objects of decimal strings,
keyed by the flattened R1CS variable names. Call `circuit.dispose()` to release
a handle, and `disposeGnark()` to stop the runtime and invalidate all handles.
The one-shot `generateGnarkProof` and `verifyGnarkProof` functions are also available.
`initGnark({ startupTimeoutMs })` optionally sets the startup deadline; the default
is 120 seconds. A failed startup can be retried on the same page.

## Updating generated applications

`go.mod` and `go.sum` pin the dependencies. Generated applications own a source
snapshot so custom hints can be registered before compilation. Updating only
the CLI does not update an existing snapshot. Generate a separate app with the
new CLI, review the `gnark-web/` diff, and update the component together while
reapplying custom hint imports. Rebuild and deploy all runtime assets together.
The public JavaScript API is the supported application interface.

Before publishing a CLI release with this adapter, publish its matching
`mopro-ffi` helper and update the scaffold's normal version dependency.
Development apps and CI can explicitly patch `mopro-ffi` to the checkout under
test. The CLI does not insert a personal fork dependency.

## Validation

From this directory, with Go and Node.js 22+ installed:

```sh
go test -count=1 ./...
node --test test/runtime.test.mjs
```

The Go tests separately compile the shipping WASM module to verify built-in hint
registration, bounded proof decoding, incompatible-key rejection and recovery.
Proof decoding accepts compressed and raw encodings using gnark's point decoder;
commitment counts and available bytes are checked before allocating vectors.

Run `benchmark/check-package.cjs` from `MoproWasmBindings/` to check the npm
package. An optional destination installs the actual tarball for browser tests.

Vite consumers must configure `worker: { format: "es" }`. Generate the solver
fixture with `go run ./cmd/benchmark -mimc -rounds 8 -out ../web/assets/gnark-solver`,
then run `npm ci` and `npm test -- /absolute/path/to/generated/web` from
`test/bundler/`. The web project's npm dependencies must also be installed.
The production bundle test runs under a URL subpath, checks startup timeout and
recovery, and generates proofs with and without isolation headers. CI verifies
those reports independently with native gnark.

Run `benchmark/browser.cjs` from the generated `web/` directory for static-module
proofs and lifecycle checks. `MOPRO_GNARK_BENCH_BASE` selects the fixture directory;
`MOPRO_GNARK_MODE=portable` asserts that isolation headers are absent. The fixture
generator supports ordinary R1CS, MiMC, commitments and built-in binary hints.
Keep build outputs outside source templates; CLI staging excludes generated
artifact directories before embedding the snapshot.
