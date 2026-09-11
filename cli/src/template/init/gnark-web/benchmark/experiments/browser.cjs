// Run from the generated web directory, after npm install and npm start.
const { createRequire } = require("node:module");
const path = require("node:path");
const fs = require("node:fs");
const localRequire = createRequire(path.resolve("package.json"));
const { Builder } = localRequire("selenium-webdriver");
const chrome = localRequire("selenium-webdriver/chrome");

(async () => {
    const mode = process.env.MOPRO_GNARK_MODE || "go";
    if (!["go", "rust", "portable"].includes(mode)) throw new Error("Unknown MOPRO_GNARK_MODE");
    const samples = Number(process.env.MOPRO_GNARK_BENCH_SAMPLES || 9);
    const warmups = Number(process.env.MOPRO_GNARK_BENCH_WARMUPS || 1);
    const solver = process.env.MOPRO_GNARK_EXPECT_SOLVER || "go";
    if (!Number.isInteger(samples) || samples < 9 || samples > 1000) throw new Error("Samples must be an integer from 9 to 1000");
    if (!Number.isInteger(warmups) || warmups < 1 || warmups > 1000) throw new Error("Warmups must be an integer from 1 to 1000");
    if (!["go", "rust"].includes(solver) || (solver === "rust" && mode !== "rust")) throw new Error("Invalid expected solver");
    const options = new chrome.Options();
    if (process.env.CHROME_BIN) options.setChromeBinaryPath(process.env.CHROME_BIN);
    options.addArguments("--headless", "--no-sandbox");
    const builder = new Builder().forBrowser("chrome").setChromeOptions(options);
    if (process.env.CHROMEDRIVER_BIN) builder.setChromeService(new chrome.ServiceBuilder(process.env.CHROMEDRIVER_BIN));
    const driver = await builder.build();
    try {
        await driver.manage().setTimeouts({ script: 240000 });
        await driver.get(process.env.MOPRO_GNARK_BENCH_URL || "http://localhost:3000/gnark-benchmark.html");
        const result = await driver.executeAsyncScript(function (mode, threads, base, count, warmups, solver, expectedBackend, done) {
            (async () => {
                const api = await import("./MoproWasmBindings/gnark/gnark.js");
                const fixture = await (await fetch(base + "fixture.json")).json();
                const load = async name => {
                    const response = await fetch(base + name);
                    if (!response.ok) throw new Error(`Cannot load ${name}: ${response.status}`);
                    return new Uint8Array(await response.arrayBuffer());
                };
                const [cs, pk, vk] = await Promise.all(["circuit.r1cs", "circuit.pk", "circuit.vk"].map(load));
                const timed = async fn => { const t = performance.now(); const value = await fn(); return { ms: performance.now() - t, value }; };
                const start = await timed(() => api.initGnark(mode === "go" ? {} : {
                    experimental: true, ...(threads === null ? {} : { threads }),
                }));
                if (mode === "rust" && (!crossOriginIsolated || start.value.threads <= 0)) {
                    throw new Error("Rust test requires isolation and an active arithmetic pool");
                }
                if (mode === "portable" && crossOriginIsolated) throw new Error("Portable test must run without isolation headers");
                if (mode !== "rust" && start.value.threads !== 0) throw new Error("Go test unexpectedly started arithmetic workers");
                if (mode === "rust" && fixture.constraints < 1024) throw new Error("Fixture is too small to exercise Rust arithmetic");
                const expected = {
                    arithmetic: mode === "rust" ? "rust" : "go",
                    solver,
                };
                const assertExecution = proof => {
                    for (const key of ["arithmetic", "solver"]) {
                        if (proof.execution?.[key] !== expected[key]) {
                            throw new Error(`Expected ${key}=${expected[key]}, got ${proof.execution?.[key]}`);
                        }
                    }
                };
                if (mode === "rust" && start.value.backend !== expectedBackend) throw new Error(`Backend mismatch: ${start.value.backend}`);
                const setup = await timed(() => api.prepareGnarkCircuit(cs, { provingKey: pk, verifyingKey: vk }));
                const circuit = setup.value;
                const verifier = await api.prepareGnarkCircuit(cs, { verifyingKey: vk });
                pk.fill(0); vk.fill(0); cs.fill(0);
                const check = async proof => {
                    assertExecution(proof);
                    if (!await verifier.verify(proof)) throw new Error("Proof verification failed");
                };
                const first = await timed(() => circuit.prove(fixture.inputs[0]));
                const warmup = first.value;
                await check(warmup);
                for (let i = 1; i < warmups; i++) await check(await circuit.prove(fixture.inputs[0]));
                let failed = false;
                try { await circuit.prove({ X: "3", Y: "1" }); } catch { failed = true; }
                if (!failed) throw new Error("Accepted an unsatisfied witness");
                const samples = [], proofs = [];
                for (let i = 0; i < count; i++) {
                    const proof = await timed(() => circuit.prove(fixture.inputs[0]));
                    await check(proof.value);
                    samples.push(proof.ms); proofs.push(proof.value);
                }
                for (const input of fixture.inputs.slice(1)) {
                    const proof = await circuit.prove(input);
                    await check(proof); proofs.push(proof);
                }
                await Promise.all([circuit.dispose(), circuit.dispose()]);
                failed = false;
                try { await circuit.prove(fixture.inputs[0]); } catch { failed = true; }
                if (!failed || !await verifier.verify(warmup)) throw new Error("Circuit disposal failed");
                await verifier.dispose(); api.disposeGnark();
                const sorted = [...samples].sort((a, b) => a - b);
                return { backend: start.value.backend, pools: start.value.pools, firstProofMs: first.ms, userAgent: navigator.userAgent, mode, execution: expected, isolated: crossOriginIsolated,
                    hardwareConcurrency: navigator.hardwareConcurrency, fixture, threads: start.value.threads,
                    warmups, startupMs: start.ms, prepareMs: setup.ms, proveMs: samples,
                    medianProveMs: (sorted[Math.floor((count - 1) / 2)] + sorted[Math.floor(count / 2)]) / 2, proofs };
            })().then(done, e => done({ error: String(e), stack: e.stack }));
        }, mode, process.env.MOPRO_GNARK_THREADS === undefined ? null : Number(process.env.MOPRO_GNARK_THREADS),
        process.env.MOPRO_GNARK_BENCH_BASE || "./assets/gnark-bench/", samples, warmups, solver, process.env.MOPRO_GNARK_BENCH_BACKEND || "arkworks");
        fs.writeFileSync(process.env.MOPRO_GNARK_BENCH_REPORT || "gnark-benchmark.json", JSON.stringify(result, null, 2));
        const { proofs, ...summary } = result;
        console.log(JSON.stringify(summary, null, 2));
        if (result.error) process.exitCode = 1;
    } finally { await driver.quit(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
