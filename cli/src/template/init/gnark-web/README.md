# Gnark browser runtime

Default builds include only the Go gnark prover and verifier. To include the
experimental Rust arithmetic and witness solver, add this to the application's
root `Cargo.toml` before running `mopro build --platforms web`:

```toml
[package.metadata.mopro.gnark]
experimental-accelerator = true
```

This setting also applies to `cargo run --bin web`. Omitting it or setting it to
`false` skips the accelerator build and excludes its files and imports from
`MoproWasmBindings/gnark/`. The existing top-level Mopro WASM build and its Rust
toolchain requirements still apply. Rebuilding replaces the entire bindings
output, so disabling the option removes previously built accelerator assets.

An accelerated package still defaults to Go at runtime.
`initGnark({ experimental: true })` enables Rust when shared memory is available;
it rejects with a rebuild instruction if the package was built without Rust.

Custom field multiplication, MSMs, FFTs and solver optimizations have
compatibility tests; they are not presented as independently audited primitives.
An opt-in does not force an unsupported circuit onto the Rust solver: hints,
commitments, custom blueprints and other unsupported plans retain Go solving.
Every browser-generated proof reports the arithmetic and solver that actually
ran in `result.execution`. Verification uses the proof and public witness only.

## Component boundary and upgrades

`go.mod` / `go.sum` pin gnark 0.14 and gnark-crypto 0.19. The accelerator is a
separate Rust workspace with its own `Cargo.lock`; its modified ark-bn254 source
retains the upstream licenses and reference configurations used by the tests.
The Go/Rust interface is internal. Its field representation and solver program
version must be changed and tested together. The JavaScript API is the supported
application boundary; direct use of `__moproGnark` or `accelerator/Key` bypasses it.

Generated applications own a source snapshot so custom hints can be registered
before compilation. Updating only the CLI does not update an existing snapshot.
To upgrade, generate a separate app with the new CLI, review the `gnark-web/`
diff, and replace this component as a unit while reapplying custom hint imports.
Keep both dependency locks and run the checks below with the application's own
circuits as well. Rebuild and deploy all of `MoproWasmBindings/gnark/` together;
do not replace one Wasm file or reuse an older `wasm_exec.js` or Rust kernel.
The Go runtime shim must come from the compiler selected by this module.

Before publishing a CLI release with this adapter, publish its matching
`mopro-ffi` helper and update the scaffold's normal version dependency. Development
apps and CI can explicitly patch `mopro-ffi` to the checkout under test. The CLI
does not add a personal fork dependency or silently change native dependencies.

## Validation

From this directory, with Node.js 22+ and Go installed:

```sh
go test -count=1 ./...
node --test test/runtime.test.mjs
cargo fmt --manifest-path accelerator/Cargo.toml --all -- --check
cargo clippy --manifest-path accelerator/Cargo.toml --workspace --all-targets --locked -- -D warnings
```

For the cross-language solver test, set `MOPRO_GNARK_SOLVER_FIXTURES` to the same
temporary directory for `go test` and
`cargo test --manifest-path accelerator/Cargo.toml --workspace --release --locked`.
Inside the Mopro repository, nested manifests use `Cargo.toml.template` so
Cargo includes their directories in the published CLI crate. The CLI build
script renders them and excludes build outputs before embedding. To check this
source snapshot directly, stage it outside the template first:

```sh
# From the Mopro repository root:
rustc --edition=2021 cli/build.rs -o /tmp/mopro-stage-templates
(cd cli && OUT_DIR=/tmp/mopro-gnark-templates /tmp/mopro-stage-templates)
cargo fmt --manifest-path /tmp/mopro-gnark-templates/init-template/gnark-web/accelerator/Cargo.toml --all -- --check
cargo clippy --manifest-path /tmp/mopro-gnark-templates/init-template/gnark-web/accelerator/Cargo.toml --workspace --all-targets --locked -- -D warnings
```

Keep `CARGO_TARGET_DIR` outside source templates when running builds. Generated
applications have ordinary `Cargo.toml` manifests and need no staging step.

The browser integration checks must use the generated package and assert proof
execution, not just successful verification. CI covers:

- Go-only packages built with the accelerator source removed, with no accelerator
  files or imports in the npm tarball or production bundle.
- Default Go proving on an isolated page in both build variants.
- Experimental Rust arithmetic and Rust solving on a hint-free circuit.
- Experimental Rust arithmetic with Go solving for commitments and built-in hints.
- Experimental opt-in on a page without isolation headers, which must use Go.
- Invalid witnesses, changed witnesses, disposal and native verification of reports.
- A production Vite bundle under a URL subpath, with Go/Rust execution assertions
  and recovery after deliberately breaking nested worker startup.

Run `benchmark/browser.cjs` from the generated `web/` directory. Select
`MOPRO_GNARK_MODE=go`, `rust`, or `portable`; `portable` requires a server without
isolation headers. `MOPRO_GNARK_BENCH_BASE` selects the fixture directory.
A Rust run fails if isolation, the arithmetic pool, or the expected solver is
missing. All reports retain execution assertions beside the proof samples.

Vite consumers must configure `worker: { format: "es" }`; the default IIFE format
cannot represent this asynchronous module runtime. The regression in
`test/bundler/` builds the packed bindings and writes Go and Rust proof reports
for independent native verification. Run `npm ci` there, then
`npm test -- /absolute/path/to/generated/web` after creating the solver fixture.
Set `MOPRO_GNARK_ACCELERATOR=true` for an accelerated package; the default checks
a Go-only package. Use the same setting for `benchmark/check-package.cjs`.
New runtimes have a 120-second startup deadline, configurable with
`initGnark({ startupTimeoutMs })`. A startup timeout rejects pending requests and
terminates the worker so a later call can start a fresh runtime.
