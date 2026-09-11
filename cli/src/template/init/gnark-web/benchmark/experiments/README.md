# Browser arithmetic experiments

This branch adds benchmark instrumentation and an asynchronous arithmetic bridge.
The existing Arkworks branch remains the deployment baseline. Timing fields are
local diagnostics, are nested (not additive), and are not proof statements.

`browser.cjs` records full proof times, first-proof latency, preparation time,
worker pools and per-proof phase timings. Verification runs outside proof timing.
`cmd/reencode` converts existing keys with `WriteRawTo` and checks an exact
compressed round trip. It never creates another setup or skips validation.

JavaScript backend comparisons use the same Go binary, fixtures and API. Worker
requests are serialized even when a backend yields. Hybrid Rust helpers are
compiled only with the `hybrid` feature; `bench-profile` enables Rust timers.
The JavaScript libraries in this directory are benchmark dependencies, not
production package defaults. See each backend branch and [the results report](RESULTS.md) for
reproduction commands and limitations.

## Reproduce

Check out the comparison and three backend branches as separate worktrees. Use
Go 1.24.0, Rust nightly-2025-11-15 with `rust-src`, wasm-pack 0.15.0, Node 24,
and matching Chrome/ChromeDriver executables. `build.py` takes the three backend
worktrees, an existing generated `MoproWasmBindings` package, the benchmark
fixtures, and a directory containing `selenium-webdriver` as arguments:

```
python3 build.py /tmp/gnark-comparison \
  --montgomery /path/to/montgomery-worktree \
  --ffjavascript /path/to/ffjavascript-worktree \
  --tuning /path/to/arkworks-tuning-worktree \
  --fixtures /path/to/fixtures \
  --bindings /path/to/MoproWasmBindings \
  --selenium /path/to/web/node_modules \
  --wasm-pack /path/to/wasm-pack
python3 serve.py /tmp/gnark-comparison
```

In a second terminal, set `CHROME_BIN` and `CHROMEDRIVER_BIN`, then run:

```
python3 run.py /tmp/gnark-comparison checks --verifier /tmp/gnark-comparison/verify.test
python3 run.py /tmp/gnark-comparison bench --verifier /tmp/gnark-comparison/verify.test
python3 run.py /tmp/gnark-comparison workers --verifier /tmp/gnark-comparison/verify.test
python3 run.py /tmp/gnark-comparison workers --threads 32 --sessions 3 --samples 31 --verifier /tmp/gnark-comparison/verify.test
python3 run.py /tmp/gnark-comparison keys --verifier /tmp/gnark-comparison/verify.test
python3 summarize.py /tmp/gnark-comparison
```

The fixture directory must include `square` (16,384 squarings), `mimc` (64 words),
`commitments` (2,048 squarings, two commitments over intermediate values), and
`hints` (the built-in-hint integration fixture). Generate them with the existing
`cmd/benchmark` command. Raw and compressed variants are the same keys; converting
never reruns trusted setup. Serve only the public synthetic benchmark fixtures.

`bench` uses three fresh sessions, five warmups and 31 timed proofs per circuit
and backend, rotating backend order. `workers` is an exploratory 1/2/4/8/16-thread
sweep with nine samples after five warmups. `keys` alternates compressed/raw
preparation across three sessions. The runner records the exact served JS/WASM
and fixture hashes and verifies every recorded proof in native gnark. Do not run
compilation, another benchmark or other CPU-intensive work during measurements.

These are experimental adapters, not a new production backend selection API.
In particular, the hybrid uses two distinct worker pools, ffjavascript lacks a
public worker-count option, and the GPL dependencies are isolated to the
ffjavascript benchmark. No backend selection changes the gnark circuit format,
proof format, solver, random blinding or commitment protocol.

For the actual npm tarball + Vite production integration checks:

```
cd bundler && npm ci && cd ..
python3 pack.py /tmp/gnark-comparison --verifier /tmp/gnark-comparison/verify.test
```

Run packaging tests outside the timing runs. They cover the unaccelerated path,
the selected accelerator, startup failure/recovery, built-in hints and independent
native verification of the resulting proofs. The ffjavascript startup fault is
injected at worker creation because that library embeds its worker source as a
blob. Rayon startup faults are injected into its fetched worker module.

For fallback on a page without cross-origin isolation, start a plain server in a
separate terminal, then run the portable stage:

```
python3 -m http.server 3218 --bind 127.0.0.1 --directory /tmp/gnark-comparison
# In another terminal:
python3 run.py /tmp/gnark-comparison portable --port 3218 --verifier /tmp/gnark-comparison/verify.test
```

This requests acceleration and asserts that initialization falls back to Go with
zero accelerator threads, then proves and independently verifies the hint fixture.
