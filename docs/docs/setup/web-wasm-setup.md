# Web(Wasm) setup

This tutorial will show you how to build static library for web browser.

Before proceeding, ensure that **Rust**, **Wasm-Pack** and **Chrome** are installed. Refer to the [Prerequisites](/docs/prerequisites).

## Support Halo2 Circuit Implementation

This section assumes the existence of a user-defined circuit implementation based on the [PSE Halo2](https://github.com/privacy-scaling-explorations/halo2).

Note that there are multiple Halo2 implementations (e.g., Zcash, PSE, Axiom). Mopro primarily supports the [PSE Halo2](https://github.com/privacy-scaling-explorations/halo2), which is a Plonk backend and works well with [wasm-bindgen-rayon](https://github.com/RReverser/wasm-bindgen-rayon). To enable multithreading in WASM, `wasm-bindgen-rayon` must be used for Halo2.

> Usage with WebAssembly
> By default, when building to WebAssembly, Rayon will treat it as any other platform without multithreading support and will fall back to sequential iteration. This allows existing code to compile and run successfully with no changes necessary, but it will run slower as it will only use a single CPU core.

from: [Rayon - github](https://github.com/rayon-rs/rayon#usage-with-webassembly)

## Update `MoproWasmBindings`

Once followed [`mopro init`](/docs/getting-started#2-initialize-adapters) in ["Getting Started"](/docs/getting-started.md) page, there is a Rust project that lets you customize your functions.

### 1. Modify 'Cargo.toml' in the root

The user-defined circuit implementation crate should be added as a dependency manually, as illustrated below:

```toml
[dependencies]
mopro-ffi = { ... }
# ...
# HALO2_DEPENDENCIES
my-halo2-circuit = { git = "http://github.com/users/my-halo2-circuit.git" }
```

### 2. Create Wrapper Functions for Generate/Verify proof method

To compile Wasm code with the circuit, wrapper functions for generating and verifying proof methods in the user-defined circuit implementation must be created in `src/lib.rs`, using the example structure provided below:

```rust
set_halo2_circuits! {
    ("my_halo2_circuit_pk.bin", my_halo2_circuit::prove, "my_halo2_circuit_vk.bin", my_halo2_circuit::verify),
}
```

### 3. Build again for web

(Optional) To ensure a clean build, remove the existing `MoproWasmBindings` directory in the `mopro-example-app`.
Then, execute the `mopro build` command again, selecting the "web" platform in `mopro-example-app`:

```shell
mopro-example-app $ rm -rf MoproWasmBindings
mopro-example-app $ mopro build
```

### 4. **Integrate the Wasm Code**:

The wasm code can be imported and used in a web application, as illustrated below:

```javascript
const mopro_wasm = await import("../MoproWasmBindings/mopro_wasm_lib.js");
await mopro_wasm.default();
await mopro_wasm.initThreadPool(navigator.hardwareConcurrency);

async function fetchBinaryFile(url) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Failed to load ${url}`);
    return new Uint8Array(await response.arrayBuffer());
}

async function generateProof(input) {
    const name = "my_halo2_circuit_pk";
    const SRS_KEY = await fetchBinaryFile("./assets/plonk_fibonacci_srs.bin");
    const PROVING_KEY = await fetchBinaryFile(
        "./assets/plonk_fibonacci_pk.bin"
    );
    const proof = await mopro_wasm.generateHalo2Proof(
        name,
        srs_key,
        proving_key,
        input
    );
    console.log(proof);
}
```

Initializing with `initThreadPool` is necessary to enable multi-threading n WebAssembly within the browser.

The generated web template runs the selected adapters' examples: Halo2 Fibonacci circuits and the gnark cubic circuit. Modify `test_mopro.js` and `index.html` to use your own circuits.

## Gnark (Groth16, BN254)

The gnark web adapter runs the Go prover and verifier in a dedicated Web Worker. The native `rust-gnark` C bridge is excluded from Rust WASM builds. Proving and verification happen locally in the browser.

Install **Go 1.24 or newer**, in addition to the Rust/WASM prerequisites above.
Until a published `mopro-ffi` release includes the web build helper, install the
CLI from a checkout containing this change and create the app outside that checkout:

```sh
# From the Mopro checkout:
cargo install --path cli --locked
cd ..
mopro init --adapter gnark --project-name gnark-app
cd gnark-app
```

For this development build, explicitly add the following to the generated
`Cargo.toml`, using the absolute path to the same checkout:

```toml
[patch.crates-io]
mopro-ffi = { path = "/absolute/path/to/mopro/mopro-ffi" }
```

The scaffold itself keeps the standard crates.io dependency and does not insert
Git or path overrides. Native gnark apps do not need this development override.
CI explicitly patches generated test apps to the checkout under review. Once the
build helper is released, select that release and remove the development patch.
Keep the generated `Cargo.lock` in version control.

```sh
mopro build --mode release --platforms web --no-auto-update
mopro create --framework web
cd web
yarn
yarn start
```

`mopro init` includes a `gnark-web/` Go module. `mopro build` compiles the Go module and packages its WASM, matching `wasm_exec.js`, worker, JavaScript API and TypeScript declarations under `MoproWasmBindings/gnark/`. Keep these files together when deploying. The main `mopro_wasm_lib.js` module also re-exports the gnark functions.

```js
import {
    prepareGnarkCircuit,
    disposeGnark,
} from "./MoproWasmBindings/gnark/gnark.js";

async function loadBytes(url) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Cannot load ${url}`);
    return new Uint8Array(await response.arrayBuffer());
}

const [r1cs, pk, vk] = await Promise.all([
    loadBytes("./assets/cubic_circuit.r1cs"),
    loadBytes("./assets/cubic_circuit.pk"),
    loadBytes("./assets/cubic_circuit.vk"),
]);

// Initialize the worker and load/validate the circuit and keys once.
const circuit = await prepareGnarkCircuit(r1cs, { provingKey: pk, verifyingKey: vk });
try {
    for (const witness of [{ X: "3", Y: "35" }, { X: "4", Y: "73" }]) {
        // Reuses the decoded circuit and keys. Only the witness is sent.
        const result = await circuit.prove(witness);
        console.log(await circuit.verify(result));
    }
} finally {
    await circuit.dispose(); // Releases this circuit's retained references.
    disposeGnark(); // Stop the runtime when the app is finished with ALL circuits.
}
```

The browser API accepts `Uint8Array` circuit and key data instead of filesystem paths. Witness values must be decimal strings, keyed by the flattened variable names stored in the R1CS, or a JSON string encoding that object. This preserves field elements larger than JavaScript's safe integer range. The result contains `proof` and `public_inputs` hex strings, compatible with the native adapter. Invalid proofs return `false`; malformed data, missing inputs and unsatisfied circuits reject the Promise.

Keep a prepared circuit alive across repeated operations. Preparation copies and decodes its source buffers once and retains the decoded R1CS and keys in Go. Later `prove()` calls send only witness JSON; `verify()` sends only proof/public-input strings. Each proof uses a fresh witness. The source buffers can be released or reused after preparation completes.

Preparation checks circuit/key dimensions, including commitment metadata, before registering a handle or starting proof work. Incompatible dimensions reject the Promise and leave existing handles usable. These checks do not establish that keys with matching dimensions came from the same setup.

Supply a `provingKey`, a `verifyingKey`, or both. For example, `prepareGnarkCircuit(r1cs, { verifyingKey: vk })` prepares a verifier without loading a proving key. Calling an operation whose key was omitted rejects the Promise.

`circuit.dispose()` is idempotent and releases that circuit's references after earlier queued operations finish. Other prepared circuits remain usable. Go's garbage collector reclaims unreachable objects; releasing a circuit does not shrink the worker's WASM memory immediately. `disposeGnark()` terminates the worker, cancels pending requests and invalidates every handle. Prepare new handles after restarting the runtime. Avoid calling it between proofs when you want to reuse the runtime and keys.

The original `generateGnarkProof(r1cs, pk, witness)` and `verifyGnarkProof(r1cs, vk, result)` functions remain available for one-shot use. They load and validate the supplied circuit and key on every call. Prefer prepared circuits for repeated operations, particularly with large keys. The demo displays setup time separately from proving and verification time. A native benchmark of the same code paths is available with `cd gnark-web && go test -run '^$' -bench BenchmarkCubic -benchmem`.

This adapter uses **gnark 0.14.0 / gnark-crypto 0.19.0**, matching `rust-gnark 0.0.2`. Use circuit/key files generated by those versions. Standard gnark hints, including `api.ToBinary`, are registered in the shipped module. Custom solver hints must be registered in the `gnark-web` Go module before rebuilding. This integration supports Groth16 over BN254; other curves and PLONK are not exposed.

Serve the files over HTTP(S). The gnark module requires WebAssembly, module
workers and Web Crypto. It does not require `SharedArrayBuffer`, cross-origin
isolation, or Halo2's `initThreadPool`. The top-level Mopro WASM build still uses
the Rust/WASM prerequisites above. Chromium is covered by CI; Firefox, WebKit
and iOS behavior are not yet validated by this integration.

New runtime initialization has a 120-second deadline. Set `startupTimeoutMs` in
`initGnark()` to change it, for example `await initGnark({ startupTimeoutMs: 30000 })`.
A startup failure or timeout terminates the worker and rejects pending requests;
a later call starts a fresh runtime. The deadline applies to startup, not proving.

Prepared keys avoid repeated decoding but retain the initial key-validation cost.
Circuit and key artifacts must use the pinned gnark version. Verification checks
proof commitment counts and available encoded bytes before allocating vectors,
and uses gnark's point decoding and subgroup checks. Compressed and raw proof
encodings are supported; truncated or trailing data rejects the Promise.

Run `go test -count=1 ./...` from `gnark-web/` with Node.js 22 or newer installed. The tests build the production Go Wasm module and exercise built-in hints, bounded proof decoding, mismatched-key rejection and recovery under Node, in addition to the native tests.

### Bundled applications

The generated example serves ES modules directly. When importing the bindings
into a Vite application, configure ES-module workers in `vite.config.js`:

```js
export default {
    worker: { format: "es" },
};
```

The default IIFE worker output does not support this asynchronous module runtime.
Import the generated `gnark/gnark.js` entry point; Vite
bundles its workers and emits the Wasm assets. Configure Vite's `base` normally
when deploying under a subpath.

The production integration test builds these imports, proves with and without
isolation headers, and checks recovery after a stalled WASM download. After generating
the solver fixture used by CI (`go run ./cmd/benchmark -mimc -rounds 8 -out
../web/assets/gnark-solver` from `gnark-web/`), run:

```sh
cd gnark-web/test/bundler
npm ci
npm test -- /absolute/path/to/generated/web
```

The generated `web/` directory must have its npm dependencies installed. The test
writes proof reports there for independent native verification.

### Reproduce the browser/native benchmark

From the generated project, create fresh test fixtures, then start the web server:

```sh
cd gnark-web
go run ./cmd/benchmark -rounds 16384
cd ../web
npm install
npm start
```

In another terminal, run from `web/`:

```sh
node ../gnark-web/benchmark/browser.cjs
```

Set `CHROME_BIN` and `CHROMEDRIVER_BIN` if needed. The script records startup, preparation and nine complete warm proofs in `web/gnark-benchmark.json`. It also checks different witnesses, invalid-input recovery, verification and disposal. It uses the same initial witness for every timed sample. Run the native comparison and independently verify the browser proofs from `gnark-web/`:

```sh
go test -run '^$' -bench BenchmarkWebFixture -benchtime=9x -benchmem
MOPRO_GNARK_BENCH_REPORT=../web/gnark-benchmark.json go test -run TestBrowserBenchmarkProofs -v
```

Generate a different workload with `go run ./cmd/benchmark -rounds 2048 -commitments 2 -commit-all` or `go run ./cmd/benchmark -mimc -rounds 64`, then repeat both measurements. Regenerating fixtures replaces their keys, so verify each browser report before generating the next set. These fixture keys are for testing only.

Add `-uncompressed` to the fixture command to measure preparation with uncompressed keys. Startup and preparation measurements exclude the script's initial asset downloads.
