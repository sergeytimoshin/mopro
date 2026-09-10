let worker;
let initialization;
let threadSetting;
let experimentalSetting;
let nextId = 0;
const pending = new Map();

/** Stop the prover and reject outstanding requests. A later call starts a new worker. */
export function disposeGnark() {
    fail(new Error("Gnark worker disposed"));
}

function fail(error) {
    worker?.terminate();
    worker = undefined;
    initialization = undefined;
    threadSetting = undefined;
    experimentalSetting = undefined;
    for (const request of pending.values()) request.reject(error);
    pending.clear();
}

/** Initialize once. Go is the default; experimental: true enables the Rust engine. */
export function initGnark(options = {}) {
    const threads = options?.threads;
    const experimental = options?.experimental;
    if (experimental !== undefined && typeof experimental !== "boolean") {
        return Promise.reject(new Error("Gnark experimental must be a boolean"));
    }
    if (threads !== undefined && (!Number.isInteger(threads) || threads < 0 || threads > 64)) {
        return Promise.reject(new Error("Gnark threads must be an integer from 0 to 64"));
    }
    if (initialization && experimental !== undefined && experimental !== experimentalSetting) {
        return Promise.reject(new Error("Dispose the gnark runtime before changing its experimental setting"));
    }
    if (threads > 0 && !(experimental ?? experimentalSetting ?? false)) {
        return Promise.reject(new Error("Arithmetic workers require experimental: true"));
    }
    if (initialization && threads !== undefined && threads !== threadSetting) {
        return Promise.reject(new Error("Dispose the gnark runtime before changing its thread setting"));
    }
    if (!initialization) {
        try {
            worker = new Worker(new URL("./gnark.worker.js", import.meta.url), { type: "module" });
        } catch (error) {
            return Promise.reject(error);
        }
        threadSetting = threads;
        experimentalSetting = experimental ?? false;
        const currentWorker = worker;
        initialization = new Promise((resolve, reject) => {
            pending.set(0, { resolve, reject });
            worker.onmessage = ({ data }) => {
                if (worker !== currentWorker) return;
                if (data.fatal) {
                    fail(new Error(data.error));
                    return;
                }
                const request = pending.get(data.id);
                if (!request) return;
                pending.delete(data.id);
                if (data.error) request.reject(new Error(data.error));
                else request.resolve(data.value);
            };
            worker.onerror = (event) => {
                if (worker === currentWorker) fail(new Error(event.message || "Gnark worker failed"));
            };
            worker.onmessageerror = () => {
                if (worker === currentWorker) fail(new Error("Cannot decode gnark worker response"));
            };
            worker.postMessage({ configure: true, threads, experimental: experimentalSetting });
        }).catch((error) => {
            if (worker === currentWorker) fail(error);
            throw error;
        });
    }
    return initialization;
}

async function call(method, args, owner) {
    // Prepared handles must never silently create or attach to a new runtime.
    if (owner && worker !== owner) throw new Error("Gnark worker disposed");
    const ready = initGnark();
    const currentWorker = worker;
    await ready;
    if (worker !== currentWorker) throw new Error("Gnark worker disposed");
    const id = ++nextId;
    return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        try {
            // Clone buffers so callers can reuse their circuit and keys.
            currentWorker.postMessage({ id, method, args });
        } catch (error) {
            pending.delete(id);
            reject(error);
        }
    });
}

function requireBytes(value, name) {
    if (!(value instanceof Uint8Array)) throw new TypeError(`${name} must be a Uint8Array`);
}

function witnessJSON(witness) {
    const json = typeof witness === "string" ? witness : JSON.stringify(witness);
    if (typeof json !== "string") throw new TypeError("witness must be an object or JSON string");
    return json;
}

function requireProof(result) {
    if (typeof result?.proof !== "string" || typeof result?.public_inputs !== "string") {
        throw new TypeError("result must contain proof and public_inputs hex strings");
    }
}

// Keep the source buffers out of the handle's closure. The decoded data lives
// in Go; subsequent requests carry only this handle and the witness or proof.
function preparedHandle(id, owner) {
    let disposed = false;
    let disposal;
    function requireLive() {
        if (disposed || worker !== owner) throw new Error("Prepared gnark circuit disposed");
    }
    return Object.freeze({
        async prove(witness) {
            requireLive();
            return call("provePrepared", [id, witnessJSON(witness)], owner);
        },
        async verify(result) {
            requireLive();
            requireProof(result);
            return call("verifyPrepared", [id, result.proof, result.public_inputs], owner);
        },
        dispose() {
            if (!disposal) {
                disposed = true;
                disposal = worker === owner
                    ? call("release", [id], owner).then(() => {}, (error) => {
                        // Terminating the worker also releases this circuit.
                        if (worker === owner) throw error;
                    })
                    : Promise.resolve();
            }
            return disposal;
        },
    });
}

/** Load and validate a circuit and its keys once for repeated proofs or verification. */
export async function prepareGnarkCircuit(r1cs, { provingKey, verifyingKey } = {}) {
    requireBytes(r1cs, "r1cs");
    if (provingKey !== undefined) requireBytes(provingKey, "provingKey");
    if (verifyingKey !== undefined) requireBytes(verifyingKey, "verifyingKey");
    if (provingKey === undefined && verifyingKey === undefined) {
        throw new TypeError("provide a provingKey, a verifyingKey, or both");
    }
    const ready = initGnark();
    const owner = worker;
    await ready;
    const id = await call("prepare", [r1cs, provingKey ?? null, verifyingKey ?? null], owner);
    if (worker !== owner) throw new Error("Gnark worker disposed");
    return preparedHandle(id, owner);
}

/** Prove a gnark Groth16 BN254 circuit using decimal strings keyed by circuit variable name. */
export async function generateGnarkProof(r1cs, provingKey, witness) {
    requireBytes(r1cs, "r1cs");
    requireBytes(provingKey, "provingKey");
    return call("prove", [r1cs, provingKey, witnessJSON(witness)]);
}

/** Verify native or browser gnark proof results. Invalid proofs return false; malformed data rejects. */
export async function verifyGnarkProof(r1cs, verifyingKey, result) {
    requireBytes(r1cs, "r1cs");
    requireBytes(verifyingKey, "verifyingKey");
    requireProof(result);
    return call("verify", [r1cs, verifyingKey, result.proof, result.public_inputs]);
}
