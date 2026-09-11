# Browser arithmetic comparison — 2026-09-11

Keep Arkworks and the existing 16-worker cap on this host. Neither replacement
library improved full-proof latency. Uncompressed keys reduced preparation time
by 57–59%, with approximately twice the proving-key transfer size. These results
support an optional key-format choice; they do not justify switching the backend.

## Full proof latency

Milliseconds; lower is better. Median of three fresh-session medians, each with
31 timed proofs after five warmups. Verification is outside the timed operation.
All variants use the same Go WASM, raw keys, witnesses and gnark protocol.

| Arithmetic implementation | Square | MiMC | Commitments |
| --- | ---: | ---: | ---: |
| Arkworks | 218.20 | 327.71 | 60.38 |
| montgomery G1 + Arkworks G2/FFT | 258.06 | 352.77 | 65.03 |
| ffjavascript / wasmcurves | 266.21 | 368.37 | 61.62 |
| Arkworks + upstream GLV adapter | 2665.52 | 3017.46 | 445.71 |
| Arkworks, serial main MSM calls | 263.82 | 367.01 | 69.21 |

Square has 16,384 squarings (16,385 constraints); MiMC has 64 input words
(21,121 constraints); the commitment circuit has 2,048 squarings and two
commitments (2,053 constraints).

At the 16-thread setting, Arkworks uses 16 Rayon workers plus the prover worker.
The hybrid uses those workers plus 15 montgomery workers. Its G1 calls run
sequentially because the pinned library shares scratch space and barriers.
G2 and FFT also precede its G1 calls. This compares complete working adapters,
not isolated implementations under identical scheduling.

The GLV adapter reuses upstream decomposition and endomorphism, then feeds twice
as many points to generic MSM, which still uses the full scalar-field bit width.
Its poor result does not establish that a specialized GLV MSM would be slower.

## Arkworks workers

Full proof medians in milliseconds. The 1–16 sweep is exploratory: one session
with nine samples per setting. The 32-worker follow-up uses three sessions of
31 samples. All sessions have five warmups.

| Workers | Square | MiMC | Commitments |
| ---: | ---: | ---: | ---: |
| 1 | 2066.91 | 2149.96 | 403.14 |
| 2 | 1069.10 | 1149.93 | 209.74 |
| 4 | 568.43 | 647.56 | 111.45 |
| 8 | 302.24 | 409.30 | 68.66 |
| 16 | 217.31 | 320.67 | 59.60 |
| 32 | 263.66 | 383.47 | 68.28 |

The 32-worker runs are slower on all three fixtures. Keep the current cap;
mobile devices require separate measurements.

## Checked key preparation

Median milliseconds over three sessions per format, alternating format order.
This excludes download time. `WriteRawTo` reencodes the existing setup; ordinary
`ReadFrom` retains curve, subgroup and key-dimension validation. Conversion also
checks an exact compressed round trip.

| Fixture | Compressed preparation | Raw preparation | Reduction | PK gzip bytes: compressed → raw |
| --- | ---: | ---: | ---: | ---: |
| square | 13963.73 | 6028.53 | 56.8% | 3,671,909 → 7,343,399 |
| mimc | 12799.48 | 5223.11 | 59.2% | 3,754,002 → 7,507,532 |
| commitments | 1861.19 | 778.46 | 58.2% | 525,765 → 1,051,201 |

Raw keys are useful when preparation dominates, especially with cached assets.
The larger first download can offset that gain on slower connections; network
performance was not measured. Raw and gzip sizes for all fixture files are in
[`comparison.json`](comparison.json).

## Arithmetic artifact sizes

Bytes for accelerator JS/WASM, including profiling and helper exports. Gzip is
summed per file. These are benchmark artifacts, not lean deployment builds.
The common Go binary is excluded: 19,843,634 bytes, or 4,123,211 bytes gzipped.
Generic Mopro bindings and shared Go/API glue are also excluded.

| Arithmetic implementation | Bytes | Gzip bytes |
| --- | ---: | ---: |
| Arkworks | 473,576 | 146,414 |
| montgomery G1 + Arkworks G2/FFT | 639,765 | 209,548 |
| ffjavascript / wasmcurves | 207,885 | 40,168 |
| Arkworks + upstream GLV adapter | 554,860 | 175,619 |
| Arkworks, serial main MSM calls | 458,959 | 142,492 |

ffjavascript has a smaller arithmetic component but was slower here, and its
GPL dependencies need separate consideration before adoption. The shared Go
WASM dominates the runtime download.

## Profile and validation

For Square with Arkworks, the median main-MSM wall time is about 192 ms, FFT
14 ms, and Rust input decoding 0.145 ms. Buffer conversion is not the leading
cost in this run. Timers are nested; concurrent per-MSM timings cannot be added.
Proving-key decoding dominates preparation.

- 90 recorded browser sessions; 2,178 proofs independently verified by native gnark.
- 90 additional native-verified proofs from actual npm tarballs through Vite production builds.
- 1,456 arithmetic comparisons at 1 and 16 workers: edge scalars, infinity points, varying MSM sizes, FFT roots and multiple live keys.
- Go tests/vet, JS runtime tests, Rust FFT/GLV differential tests and Clippy passed.
- Packed startup failure/recovery and portable Go fallback passed, including the final hybrid loader.
- No mobile, Safari or Firefox measurements. No production deployment was changed.

Linux, AMD Ryzen 9 9950X (16 cores / 32 logical processors), Chrome for Testing
153.0.8010.12, Go 1.24.0, gnark 0.14.0 / gnark-crypto 0.19.0, Arkworks 0.5,
Rust nightly-2025-11-15, wasm-pack 0.15.0, Node 24.11.0. Library versions:
montgomery 0.4.0, ffjavascript 0.3.1, wasmcurves 0.2.2. Full tool versions,
session samples, phase medians and exact artifact/fixture hashes are committed
in [`comparison.json`](comparison.json).

The initial backend runs rotated order across sessions. Final hybrid sessions
were rerun separately after fixing lazy loading for non-isolated pages; those
final sessions replace its earlier measurements. Other artifacts were unchanged.
No compilation or parallel benchmark ran during timing.

## Branches and reproduction

- [Shared harness and results](https://github.com/sergeytimoshin/mopro/tree/feat/gnark-web-browser-bench)
- [montgomery hybrid](https://github.com/sergeytimoshin/mopro/tree/feat/gnark-web-montgomery)
- [ffjavascript](https://github.com/sergeytimoshin/mopro/tree/feat/gnark-web-ffjavascript)
- [Arkworks tuning](https://github.com/sergeytimoshin/mopro/tree/feat/gnark-web-arkworks-tuning)

See [`README.md`](README.md) for build/run commands and [`NOTES.md`](NOTES.md)
for adapter constraints. These branches contain experiments; the production
backend-selection API and deployed Arkworks branch are unchanged.

The local evidence archive `mopro-gnark-browser-experiments-2026-09-11.tar.gz` contains full proof reports,
synthetic fixtures, generated runtime files, source snapshots and validation logs.
It is retained locally, not checked into Git. SHA-256:
`cc0e552d45c3738e145529044b8405d910e34f7721d5c4296683f4b8edaa4746`.
