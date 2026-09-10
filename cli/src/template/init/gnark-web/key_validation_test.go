package main

import (
	"bytes"
	"testing"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark/backend/groth16"
	native "github.com/consensys/gnark/backend/groth16/bn254"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/r1cs"
)

func TestKeyDimensions(t *testing.T) {
	cs, err := frontend.Compile(ecc.BN254.ScalarField(), r1cs.NewBuilder, &committedCircuit{})
	if err != nil {
		t.Fatal(err)
	}
	original, verifier, err := groth16.Setup(cs)
	if err != nil {
		t.Fatal(err)
	}
	circuit, pkBytes, vkBytes := serialized(t, cs), serialized(t, original), serialized(t, verifier)
	for name, change := range map[string]func(*native.ProvingKey){
		"domain":               func(pk *native.ProvingKey) { pk.Domain.Cardinality *= 2 },
		"quotient":             func(pk *native.ProvingKey) { pk.G1.Z = nil },
		"private wires":        func(pk *native.ProvingKey) { pk.G1.K = nil },
		"A points":             func(pk *native.ProvingKey) { pk.G1.A = nil },
		"B points":             func(pk *native.ProvingKey) { pk.G1.B = nil },
		"G2 points":            func(pk *native.ProvingKey) { pk.G2.B = nil },
		"infinity mask":        func(pk *native.ProvingKey) { pk.InfinityA = nil },
		"infinity count":       func(pk *native.ProvingKey) { pk.NbInfinityB++ },
		"commitments":          func(pk *native.ProvingKey) { pk.CommitmentKeys = nil },
		"commitment basis":     func(pk *native.ProvingKey) { pk.CommitmentKeys[0].Basis = nil },
		"commitment knowledge": func(pk *native.ProvingKey) { pk.CommitmentKeys[0].BasisExpSigma = nil },
	} {
		t.Run(name, func(t *testing.T) {
			pk := groth16.NewProvingKey(ecc.BN254).(*native.ProvingKey)
			if _, err := pk.ReadFrom(bytes.NewReader(pkBytes)); err != nil {
				t.Fatal(err)
			}
			change(pk)
			if _, err := prepareCircuit(circuit, serialized(t, pk), vkBytes); err == nil {
				t.Fatal("accepted incompatible proving key")
			}
		})
	}
	for name, change := range map[string]func(*native.VerifyingKey){
		"public wires":       func(vk *native.VerifyingKey) { vk.G1.K = nil },
		"commitments":        func(vk *native.VerifyingKey) { vk.CommitmentKeys = nil },
		"commitment map":     func(vk *native.VerifyingKey) { vk.PublicAndCommitmentCommitted = nil },
		"commitment indices": func(vk *native.VerifyingKey) { vk.PublicAndCommitmentCommitted[0] = []int{99} },
	} {
		t.Run(name, func(t *testing.T) {
			vk := groth16.NewVerifyingKey(ecc.BN254).(*native.VerifyingKey)
			if _, err := vk.ReadFrom(bytes.NewReader(vkBytes)); err != nil {
				t.Fatal(err)
			}
			change(vk)
			if _, err := prepareCircuit(circuit, pkBytes, serialized(t, vk)); err == nil {
				t.Fatal("accepted incompatible verifying key")
			}
		})
	}
	if _, err := prepareCircuit(circuit, pkBytes, vkBytes); err != nil {
		t.Fatalf("valid key rejected: %v", err)
	}
}
