//go:build !js || !wasm

package main

// Native builds keep gnark's own parallel prover and assembly kernels.
func prepareKernel(c *preparedCircuit) {}

func kernelProfile(kernel proofKernel) map[string]float64 { return nil }
