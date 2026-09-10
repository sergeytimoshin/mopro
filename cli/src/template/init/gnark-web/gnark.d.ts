export interface GnarkProofResult {
    /** Compressed gnark Groth16 BN254 proof, encoded as hex. */
    proof: string;
    /** Serialized gnark public witness, encoded as hex. */
    public_inputs: string;
}
export interface GnarkOptions {
    /** Arithmetic workers (1–64), or 0 for Go only. Default: reported CPU count, capped at 16. */
    threads?: number;
}
export interface GnarkRuntimeInfo {
    /** Actual arithmetic pool size; 0 when threading is disabled or unavailable. */
    threads: number;
}
/** Configure before preparing circuits. Dispose the runtime before changing settings. */
export function initGnark(options?: GnarkOptions): Promise<GnarkRuntimeInfo>;
export function disposeGnark(): void;

export interface GnarkCircuitKeys {
    provingKey?: Uint8Array;
    verifyingKey?: Uint8Array;
}

export interface GnarkCircuit {
    /** Requires a provingKey. Sends only the witness to the worker. */
    prove(witness: Record<string, string> | string): Promise<GnarkProofResult>;
    /** Requires a verifyingKey. Sends only the proof and public inputs. */
    verify(result: GnarkProofResult): Promise<boolean>;
    /** Release retained circuit/key references after earlier queued operations finish. Idempotent. */
    dispose(): Promise<void>;
}

/** Load and validate once. Supply at least one key. disposeGnark() invalidates all handles. */
export function prepareGnarkCircuit(r1cs: Uint8Array, keys: GnarkCircuitKeys): Promise<GnarkCircuit>;

/** One-shot API: loads the circuit and proving key on every call. */
export function generateGnarkProof(
    r1cs: Uint8Array,
    provingKey: Uint8Array,
    witness: Record<string, string> | string,
): Promise<GnarkProofResult>;
/** One-shot API: loads the circuit and verifying key on every call. */
export function verifyGnarkProof(
    r1cs: Uint8Array,
    verifyingKey: Uint8Array,
    result: GnarkProofResult,
): Promise<boolean>;
