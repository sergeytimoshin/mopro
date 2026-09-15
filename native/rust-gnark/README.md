# Mopro native gnark 0.15 backend

This is the Groth16-only `rust-gnark` v0.0.2 source updated to gnark v0.15.0
and gnark-crypto v0.20.1. It additionally registers gnark's standard
`rangecheck` hints, required by Zolana circuits using `api.ToBinary`.

Source: `FluxePay/rust-gnark` tag `v0.0.2` (`02029a5`). The upstream license
and README are included. No PLONK implementation or dependency is included.
