package main

import (
	"bytes"
	"encoding/binary"
	"fmt"

	curve "github.com/consensys/gnark-crypto/ecc/bn254"
	native "github.com/consensys/gnark/backend/groth16/bn254"
)

// Decode gnark 0.14's proof layout using its subgroup-checking point decoder.
// Proof.ReadFrom allocates the commitment vector from an untrusted length before
// reading its points. Check that length against the circuit and available bytes
// first. The point decoder supports both compressed and raw encodings.
func readProof(data []byte, commitments int) (*native.Proof, error) {
	reader := bytes.NewReader(data)
	decoder := curve.NewDecoder(reader)
	proof := new(native.Proof)
	for _, point := range []any{&proof.Ar, &proof.Bs, &proof.Krs} {
		if err := decoder.Decode(point); err != nil {
			return nil, err
		}
	}
	var count uint32
	if err := binary.Read(reader, binary.BigEndian, &count); err != nil {
		return nil, err
	}
	if uint64(count) != uint64(commitments) {
		return nil, fmt.Errorf("proof commitment count does not match the circuit")
	}
	// Include the final commitment PoK. Division avoids an overflowing length
	// multiplication, including on wasm32, and bounds allocation by input size.
	if uint64(count)+1 > uint64(reader.Len()/curve.SizeOfG1AffineCompressed) {
		return nil, fmt.Errorf("truncated proof commitments")
	}
	proof.Commitments = make([]curve.G1Affine, int(count))
	for i := range proof.Commitments {
		if err := decoder.Decode(&proof.Commitments[i]); err != nil {
			return nil, err
		}
	}
	if err := decoder.Decode(&proof.CommitmentPok); err != nil {
		return nil, err
	}
	if reader.Len() != 0 {
		return nil, fmt.Errorf("trailing proof data")
	}
	return proof, nil
}
