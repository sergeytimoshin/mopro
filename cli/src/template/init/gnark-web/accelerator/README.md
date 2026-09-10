# Gnark browser arithmetic

This internal WASM module accelerates the quotient polynomial and the five main
Groth16 MSMs, plus large commitment MSMs. It also solves ordinary hint-free
R1CS circuits. Go retains safe key decoding, the commitment protocol, random blinding, proof serialization and
verification. The wire format uses the same
four little-endian Montgomery limbs as gnark-crypto 0.19 and arkworks 0.5.

The solver prepares gnark's dependency order once and checks every term, wire,
coefficient and constraint row before use. Constant expressions are folded and
shared prefixes of linear expressions are evaluated once per proof. The plan
schedules each shared sum at its first use, when all of its wires are known;
cached values belong to that proof only. Simple wire references avoid general
expression evaluation. Solved vectors feed
the FFT and MSMs directly, without a round trip through Go. Each proof uses fresh
witness storage and rejects unsatisfied constraints. Hints, custom blueprints,
commitments, GKR, circuit logging and custom solver options keep gnark's solver.
An unsupported plan falls back at preparation; witness failures remain errors.
The solver follows gnark 0.14.0's `solveR1C` semantics under Apache-2.0.

The worker starts a shared-memory Rayon pool only on cross-origin isolated pages.
Other pages retain the Go prover. Small circuits also use Go. This kernel never
receives witnesses or keys from a remote service.

`ark-bn254` is vendored from crates.io 0.5.0 (MIT OR Apache-2.0). Its field
configuration delegates multiplication and squaring to an unrolled mixed-radix
implementation while preserving constants and the 2^256 Montgomery encoding.
Pairs of base-field products share one Montgomery reduction. Reference field
configurations are retained for differential tests. Other curve code is upstream.

Prepared keys cache FFT twiddles and coset weights. Paired DIF/DIT transforms
avoid intermediate bit-reversal passes; the final inverse transform emits the
order used by gnark's Z key. Normalization and the quotient denominator are folded
into cached weights. Tests compare with arkworks using alternate roots, cosets,
partially filled domains and field boundary values.

Larger MSMs use signed-window Pippenger with batched affine bucket additions.
Workers split larger windows into groups of 256 buckets so idle threads can help
finish a window. Each group uses disjoint point storage and one inversion per
tree level; weighted totals restore the groups' original bucket indices. A
single-thread pool keeps the full window together. This adds no prepared key
data. G2 additions batch their
Fq2 denominator norms in the base field, then recover each inverse by scaling
the conjugate. This saves extension-field multiplications while retaining the
same point formulas and memory use. Exceptional points use
arkworks' complete projective formulas. Contiguous bucket storage avoids
contention from small allocations in threaded WASM. Scalar recoding uses indexed
parallel writes into one preallocated buffer. Differential tests compare
both groups with arkworks across window boundaries, zero/maximal scalars,
infinity, repeated points and cancellation.

The weighted bucket sum also uses affine trees: pairing adjacent buckets halves
the weights, while a batched sum of odd-weight buckets supplies the correction.
This shares inversions across levels' partial sums and leaves only a logarithmic
number of projective operations at the end.

Build through `mopro build --platforms web`. Run native arithmetic tests with
`cargo test --manifest-path gnark-web/accelerator/Cargo.toml --workspace --release`.
For the solver's cross-language differential tests, set
`MOPRO_GNARK_SOLVER_FIXTURES` to a temporary directory, run
`go test -run TestKernelSolverFixtures -count=1` in `gnark-web/`, then run the Rust tests
with the same environment variable. CI checks every witness and constraint field
against native gnark, and independently verifies browser proofs.
