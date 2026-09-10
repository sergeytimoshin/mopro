// Initialize the WASM module and thread pool
async function initializeWasm() {
    try {
        const mopro_wasm = await import('./MoproWasmBindings/mopro_wasm_lib.js');
        if (mopro_wasm.generateHalo2Proof) {
            await mopro_wasm.default();
            await mopro_wasm.initThreadPool(navigator.hardwareConcurrency);
        }
        return mopro_wasm;
    } catch (error) {
        console.error("Failed to initialize WASM module or thread pool:", error);
        throw error;
    }
}

// Fetch binary file
async function fetchBinaryFile(url) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Failed to load ${url}`);
    return new Uint8Array(await response.arrayBuffer());
}

// Measure the execution time of a given callback
async function measureTime(callback) {
    const start = performance.now();
    const result = await callback();
    const end = performance.now();
    return { result, timeTaken: (end - start).toFixed(2) }; // milliseconds
}

// Get pk and vk from the binary file
function getNameFromPath(path) {
    return path.split('/').pop();
}

// Run a specific test
async function runTest(testName, input, srs, pk, vk, generateProof, verifyProof) {
    try {
        const pkName = getNameFromPath(pk);
        const vkName = getNameFromPath(vk);
        const SRS_KEY = await fetchBinaryFile(srs);
        const PROVING_KEY = await fetchBinaryFile(pk);
        const VERIFYING_KEY = await fetchBinaryFile(vk);

        const { result: proofResult, timeTaken: proofTime } = await measureTime(() =>
            generateProof(pkName, SRS_KEY, PROVING_KEY, input)
        );

        const [proof, public_input] = proofResult;

        const { result: verifyResult, timeTaken: verifyTime } = await measureTime(() =>
            verifyProof(vkName, SRS_KEY, VERIFYING_KEY, proof, public_input)
        );

        return { isValid: verifyResult, proofTime, verifyTime };
    } catch (error) {
        console.error(`Error during ${testName} test:`, error);
        throw error;
    }
}

// Finalize the test suite and display the final status
function finalizeTests(allPassed, statusDiv) {
    const finalStatus = allPassed ? "All tests passed" : "Some tests failed";
    statusDiv.textContent = `Test Status: ${finalStatus}`;
    statusDiv.dataset.status = allPassed ? "passed" : "failed";
}

// Update the test results in the table
function updateResults(testName, data, resultsTable, allPassedRef, error = null) {
    let row = document.getElementById(testName);
    if (!row) {
        row = document.createElement('tr');
        row.id = testName;
        row.innerHTML = `
            <td>${testName}</td>
            <td id="${testName}-setup-time"></td>
            <td id="${testName}-proof-time"></td>
            <td id="${testName}-verify-time"></td>
            <td id="${testName}-pass"></td>
            <td id="${testName}-error"></td>
        `;
        resultsTable.appendChild(row);
    }

    if (error) {
        allPassedRef.value = false; // Use an object to maintain a mutable reference to `allPassed`
        document.getElementById(`${testName}-error`).textContent = error;
    } else {
        document.getElementById(`${testName}-setup-time`).textContent = data.setupTime ?? "—";
        document.getElementById(`${testName}-proof-time`).textContent = data.proofTime;
        document.getElementById(`${testName}-verify-time`).textContent = data.verifyTime;
        document.getElementById(`${testName}-pass`).textContent = data.isValid ? "true" : "false";
        if (!data.isValid) allPassedRef.value = false;
    }
}

// Main function to initialize and run the tests
(async function () {
    // Initialize WASM
    const mopro_wasm = await initializeWasm();

    const testCases = mopro_wasm.generateHalo2Proof ? [
        {
            name: "Plonk",
            input: { out: ["55"] },
            srs: './assets/plonk_fibonacci_srs.bin',
            pk: './assets/plonk_fibonacci_pk.bin',
            vk: './assets/plonk_fibonacci_vk.bin',
            generateProof: mopro_wasm.generateHalo2Proof,
            verifyProof: mopro_wasm.verifyHalo2Proof,
        },
        {
            name: "HyperPlonk",
            input: { out: ["55"] },
            srs: './assets/hyperplonk_fibonacci_srs.bin',
            pk: './assets/hyperplonk_fibonacci_pk.bin',
            vk: './assets/hyperplonk_fibonacci_vk.bin',
            generateProof: mopro_wasm.generateHalo2Proof,
            verifyProof: mopro_wasm.verifyHalo2Proof,
        },
        {
            name: "Gemini",
            input: { out: ["55"] },
            srs: './assets/gemini_fibonacci_srs.bin',
            pk: './assets/gemini_fibonacci_pk.bin',
            vk: './assets/gemini_fibonacci_vk.bin',
            generateProof: mopro_wasm.generateHalo2Proof,
            verifyProof: mopro_wasm.verifyHalo2Proof,
        }
    ] : [];

    const resultsTable = document.getElementById('test-results');
    const statusDiv = document.getElementById('test-status');

    let currentIndex = 0;
    const allPassedRef = { value: true };

    if (mopro_wasm.generateGnarkProof) {
        try {
            const data = await runGnarkTest(mopro_wasm);
            updateResults("Gnark", data, resultsTable, allPassedRef);
        } catch (error) {
            updateResults("Gnark", null, resultsTable, allPassedRef, error.message);
        } finally {
            mopro_wasm.disposeGnark();
        }
    } else if (testCases.length === 0) {
        throw new Error("No supported web proving adapter found");
    }

    // Run each test sequentially
    while (currentIndex < testCases.length) {
        const testCase = testCases[currentIndex];
        try {
            const data = await runTest(
                testCase.name,
                testCase.input,
                testCase.srs,
                testCase.pk,
                testCase.vk,
                testCase.generateProof,
                testCase.verifyProof
            );
            updateResults(testCase.name, data, resultsTable, allPassedRef);
        } catch (error) {
            updateResults(testCase.name, null, resultsTable, allPassedRef, error.message);
        }
        currentIndex++;
    }

    // Finalize the tests
    finalizeTests(allPassedRef.value, statusDiv);
})().catch((error) => {
    const status = document.getElementById('test-status');
    status.textContent = `Test failed: ${error.message}`;
    status.dataset.status = "failed";
});

async function runGnarkTest(wasm) {
    const setupStart = performance.now();
    const [r1cs, pk, vk] = await Promise.all([
        fetchBinaryFile('./assets/cubic_circuit.r1cs'),
        fetchBinaryFile('./assets/cubic_circuit.pk'),
        fetchBinaryFile('./assets/cubic_circuit.vk'),
    ]);
    // Concurrent startup callers share a single runtime.
    for (const threads of [-1, 65, 1.5]) await expectGnarkRejection(() => wasm.initGnark({ threads }));
    const [runtime, sameRuntime] = await Promise.all([wasm.initGnark({ experimental: true, threads: 2 }), wasm.initGnark()]);
    if (runtime.threads !== sameRuntime.threads || runtime.threads !== (crossOriginIsolated ? 2 : 0)) {
        throw new Error("Gnark thread configuration was not applied");
    }
    await expectGnarkRejection(() => wasm.initGnark({ threads: 3 }));
    const sources = [r1cs.slice(), pk.slice(), vk.slice()];
    const circuit = await wasm.prepareGnarkCircuit(sources[0], {
        provingKey: sources[1], verifyingKey: sources[2],
    });
    const setupTime = (performance.now() - setupStart).toFixed(2);
    // Prepared handles must own decoded data, not reuse the source buffers.
    for (const bytes of sources) bytes.fill(0);
    const { result, timeTaken: proofTime } = await measureTime(() =>
        circuit.prove({ X: "3", Y: "35" })
    );
    const { result: isValid, timeTaken: verifyTime } = await measureTime(() =>
        circuit.verify(result)
    );
    const altered = { ...result, public_inputs: result.public_inputs.slice(0, -2) + "24" };
    if (await circuit.verify(altered)) {
        throw new Error("Gnark accepted altered public inputs");
    }
    for (const invalidCall of [
        () => circuit.prove({ X: "3", Y: "36" }),
        () => circuit.prove({ Y: "35" }),
        () => circuit.verify({ ...result, proof: "invalid" }),
        () => wasm.prepareGnarkCircuit(r1cs, { provingKey: new Uint8Array(), verifyingKey: vk }),
        () => wasm.prepareGnarkCircuit(r1cs, {}),
        () => wasm.generateGnarkProof(r1cs, pk, { X: "3", Y: "36" }),
        () => wasm.generateGnarkProof(r1cs, pk, { Y: "35" }),
        () => wasm.generateGnarkProof(new Uint8Array(), pk, { X: "3", Y: "35" }),
        () => wasm.verifyGnarkProof(r1cs, vk, { ...result, proof: "invalid" }),
    ]) {
        let rejected = false;
        try { await invalidCall(); } catch { rejected = true; }
        if (!rejected) throw new Error("Gnark accepted malformed or unsatisfied input");
    }
    if (!await circuit.verify(result) || !await wasm.verifyGnarkProof(r1cs, vk, result)) {
        throw new Error("Gnark worker did not recover after invalid input");
    }
    // Queued operations use distinct witnesses on the same prepared circuit.
    const [first, second] = await Promise.all([
        circuit.prove({ X: "1", Y: "7" }),
        circuit.prove({ X: "2", Y: "15" }),
    ]);
    if (first.public_inputs === second.public_inputs ||
        !await circuit.verify(first) || !await circuit.verify(second)) {
        throw new Error("Prepared gnark circuit reused a witness");
    }
    const legacy = await wasm.generateGnarkProof(r1cs, pk, { X: "3", Y: "35" });
    if (!await circuit.verify(legacy)) throw new Error("Prepared verifier rejected a one-shot proof");
    const verifier = await wasm.prepareGnarkCircuit(r1cs, { verifyingKey: vk });
    await expectGnarkRejection(() => verifier.prove({ X: "3", Y: "35" }));
    await Promise.all([circuit.dispose(), circuit.dispose()]);
    await expectGnarkRejection(() => circuit.prove({ X: "3", Y: "35" }));
    if (!await verifier.verify(result)) throw new Error("Disposal affected another circuit");
    // New runtime handles can reuse IDs. Old handles must never reach them.
    wasm.disposeGnark();
    const newProver = await wasm.prepareGnarkCircuit(r1cs, { provingKey: pk });
    const newVerifier = await wasm.prepareGnarkCircuit(r1cs, { verifyingKey: vk });
    await expectGnarkRejection(() => verifier.verify(result));
    await verifier.dispose();
    const newProof = await newProver.prove({ X: "3", Y: "35" });
    await expectGnarkRejection(() => newProver.verify(newProof));
    if (!await newVerifier.verify(newProof)) throw new Error("Stale handle disposed a new circuit");
    await Promise.all([newProver.dispose(), newVerifier.dispose()]);
    wasm.disposeGnark();
    const cancelled = wasm.generateGnarkProof(r1cs, pk, { X: "3", Y: "35" });
    wasm.disposeGnark();
    const portable = wasm.initGnark({ threads: 0 });
    const restarted = wasm.verifyGnarkProof(r1cs, vk, result);
    if ((await portable).threads !== 0) throw new Error("Gnark ignored the Go-only setting");
    let rejected = false;
    try { await cancelled; } catch { rejected = true; }
    if (!rejected) throw new Error("Disposing gnark did not cancel startup");
    if (!await restarted) {
        throw new Error("Gnark worker did not restart after disposal");
    }
    return { isValid, setupTime, proofTime, verifyTime };
}

async function expectGnarkRejection(call) {
    try { await call(); } catch { return; }
    throw new Error("Gnark accepted an unavailable operation or a disposed circuit");
}
