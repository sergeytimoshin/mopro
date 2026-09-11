package main

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark/backend/groth16"
	native "github.com/consensys/gnark/backend/groth16/bn254"
	"github.com/consensys/gnark/backend/witness"
	"github.com/consensys/gnark/constraint"
	csbn254 "github.com/consensys/gnark/constraint/bn254"
)

// Local diagnostics, not part of the proof or its verification statement.
type proofExecution struct {
	Arithmetic string `json:"arithmetic"`
	Solver     string `json:"solver"`
}

// Keep proof and public-input serialization compatible with rust-gnark 0.0.2.
type proofResult struct {
	Proof        string         `json:"proof"`
	PublicInputs string         `json:"public_inputs"`
	Execution    proofExecution `json:"execution"`
}

// A worker uses each prepared circuit serially. Only the circuit and keys are
// retained; every proof builds a fresh witness from the caller's input.
type preparedCircuit struct {
	cs     *csbn254.R1CS
	pk     groth16.ProvingKey
	vk     groth16.VerifyingKey
	kernel proofKernel
}

func prepareCircuit(r1cs, provingKey, verifyingKey []byte) (*preparedCircuit, error) {
	if provingKey == nil && verifyingKey == nil {
		return nil, fmt.Errorf("provide a proving key, a verifying key, or both")
	}
	cs, err := readCircuit(r1cs)
	if err != nil {
		return nil, err
	}
	circuit := &preparedCircuit{cs: cs}
	if provingKey != nil {
		circuit.pk = groth16.NewProvingKey(ecc.BN254)
		if _, err := circuit.pk.ReadFrom(bytes.NewReader(provingKey)); err != nil {
			return nil, fmt.Errorf("read proving key: %w", err)
		}
	}
	if verifyingKey != nil {
		circuit.vk = groth16.NewVerifyingKey(ecc.BN254)
		if _, err := circuit.vk.ReadFrom(bytes.NewReader(verifyingKey)); err != nil {
			return nil, fmt.Errorf("read verifying key: %w", err)
		}
	}
	if err := circuit.validateKeys(); err != nil {
		return nil, err
	}
	prepareKernel(circuit)
	return circuit, nil
}

func readCircuit(data []byte) (*csbn254.R1CS, error) {
	cs := groth16.NewCS(ecc.BN254).(*csbn254.R1CS)
	if _, err := cs.ReadFrom(bytes.NewReader(data)); err != nil {
		return nil, fmt.Errorf("read R1CS: %w", err)
	}
	if len(cs.Public) == 0 || cs.Public[0] != "1" {
		return nil, fmt.Errorf("expected a BN254 R1CS with a constant public wire")
	}
	return cs, nil
}

func buildWitness(input string, cs *csbn254.R1CS) (witness.Witness, error) {
	// Decimal strings avoid rounding field elements through JavaScript numbers.
	var fields map[string]string
	if err := json.Unmarshal([]byte(input), &fields); err != nil {
		return nil, fmt.Errorf("witness must be a JSON object of decimal strings: %w", err)
	}
	values := make(chan any, len(cs.Public)-1+len(cs.Secret))
	for _, names := range [][]string{cs.Public[1:], cs.Secret} {
		for _, name := range names {
			value, ok := fields[name]
			if !ok {
				return nil, fmt.Errorf("missing witness value for %q", name)
			}
			values <- value
		}
	}
	close(values)
	w, err := witness.New(ecc.BN254.ScalarField())
	if err != nil {
		return nil, err
	}
	if err := w.Fill(len(cs.Public)-1, len(cs.Secret), values); err != nil {
		return nil, fmt.Errorf("fill witness: %w", err)
	}
	return w, nil
}

func prove(r1cs, key []byte, input string) (proofResult, error) {
	circuit, err := prepareCircuit(r1cs, key, nil)
	if err != nil {
		return proofResult{}, err
	}
	defer circuit.close()
	return circuit.prove(input)
}

func (c *preparedCircuit) prove(input string) (proofResult, error) {
	var result proofResult
	if c.pk == nil {
		return result, fmt.Errorf("this circuit was prepared without a proving key")
	}
	w, err := buildWitness(input, c.cs)
	if err != nil {
		return result, err
	}
	execution := proofExecution{Arithmetic: "go", Solver: "go"}
	var p groth16.Proof
	if c.kernel == nil {
		p, err = groth16.Prove(c.cs, c.pk, w)
	} else {
		p, err = proveAccelerated(c.cs, c.pk.(*native.ProvingKey), c.kernel, w, &execution)
	}
	if err != nil {
		return result, fmt.Errorf("generate proof: %w", err)
	}
	var proof bytes.Buffer
	if _, err := p.WriteTo(&proof); err != nil {
		return result, err
	}
	public, err := w.Public()
	if err != nil {
		return result, err
	}
	publicBytes, err := public.MarshalBinary()
	if err != nil {
		return result, err
	}
	return proofResult{Proof: hex.EncodeToString(proof.Bytes()), PublicInputs: hex.EncodeToString(publicBytes), Execution: execution}, nil
}

func verify(r1cs, key []byte, result proofResult) (bool, error) {
	circuit, err := prepareCircuit(r1cs, nil, key)
	if err != nil {
		return false, err
	}
	return circuit.verify(result)
}

func (c *preparedCircuit) verify(result proofResult) (bool, error) {
	if c.vk == nil {
		return false, fmt.Errorf("this circuit was prepared without a verifying key")
	}
	proofBytes, err := hex.DecodeString(result.Proof)
	if err != nil {
		return false, fmt.Errorf("decode proof: %w", err)
	}
	p, err := readProof(proofBytes, len(c.cs.CommitmentInfo.(constraint.Groth16Commitments)))
	if err != nil {
		return false, fmt.Errorf("read proof: %w", err)
	}
	publicBytes, err := hex.DecodeString(result.PublicInputs)
	if err != nil {
		return false, fmt.Errorf("decode public inputs: %w", err)
	}
	// A serialized public witness contains only the circuit's public elements.
	// Check its header before deserialization (which trusts the vector length).
	expected := len(c.cs.Public) - 1
	if len(publicBytes) != 12+32*expected ||
		binary.BigEndian.Uint32(publicBytes[0:4]) != uint32(expected) ||
		binary.BigEndian.Uint32(publicBytes[4:8]) != 0 ||
		binary.BigEndian.Uint32(publicBytes[8:12]) != uint32(expected) {
		return false, fmt.Errorf("public witness does not match the circuit")
	}
	w, err := witness.New(ecc.BN254.ScalarField())
	if err != nil {
		return false, err
	}
	if err := w.UnmarshalBinary(publicBytes); err != nil {
		return false, fmt.Errorf("read public inputs: %w", err)
	}
	return groth16.Verify(p, c.vk, w) == nil, nil
}

func (c *preparedCircuit) close() {
	if c.kernel != nil {
		c.kernel.close()
		c.kernel = nil
	}
}
