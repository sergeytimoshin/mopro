package main

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"testing"

	"github.com/consensys/gnark-crypto/ecc"
	curve "github.com/consensys/gnark-crypto/ecc/bn254"
	"github.com/consensys/gnark/backend/groth16"
	native "github.com/consensys/gnark/backend/groth16/bn254"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/r1cs"
)

func TestProofEncodingsAndBounds(t *testing.T) {
	for _, withCommitment := range []bool{false, true} {
		name := "cubic"
		circuit, pk, vk := vector(t, "r1cs"), vector(t, "pk"), vector(t, "vk")
		input := `{"X":"3","Y":"35"}`
		expected := 0
		if withCommitment {
			name = "commitment"
			cs, err := frontend.Compile(ecc.BN254.ScalarField(), r1cs.NewBuilder, &committedCircuit{})
			if err != nil {
				t.Fatal(err)
			}
			proving, verifying, err := groth16.Setup(cs)
			if err != nil {
				t.Fatal(err)
			}
			circuit, pk, vk = serialized(t, cs), serialized(t, proving), serialized(t, verifying)
			input, expected = `{"X":"3","Y":"9"}`, 1
		}
		t.Run(name, func(t *testing.T) {
			prepared, err := prepareCircuit(circuit, pk, vk)
			if err != nil {
				t.Fatal(err)
			}
			result, err := prepared.prove(input)
			if err != nil {
				t.Fatal(err)
			}
			compressed, err := hex.DecodeString(result.Proof)
			if err != nil {
				t.Fatal(err)
			}
			original := new(native.Proof)
			if _, err := original.ReadFrom(bytes.NewReader(compressed)); err != nil {
				t.Fatal(err)
			}
			var raw, mixed bytes.Buffer
			if _, err := original.WriteRawTo(&raw); err != nil {
				t.Fatal(err)
			}
			// Per-point encoding is supported by gnark's decoder too.
			points := []any{&original.Ar, &original.Bs, &original.Krs, original.Commitments, &original.CommitmentPok}
			for i, point := range points {
				encoder := curve.NewEncoder(&mixed)
				if i%2 == 0 {
					encoder = curve.NewEncoder(&mixed, curve.RawEncoding())
				}
				if err := encoder.Encode(point); err != nil {
					t.Fatal(err)
				}
			}
			for encoding, data := range map[string][]byte{"compressed": compressed, "raw": raw.Bytes(), "mixed": mixed.Bytes()} {
				t.Run(encoding, func(t *testing.T) {
					result.Proof = hex.EncodeToString(data)
					if ok, err := prepared.verify(result); err != nil || !ok {
						t.Fatalf("valid proof rejected: valid=%v err=%v", ok, err)
					}
					for _, cut := range []int{0, 1, len(data) / 2, len(data) - 1} {
						if _, err := readProof(data[:cut], expected); err == nil {
							t.Fatalf("accepted truncated proof at %d", cut)
						}
					}
					if _, err := readProof(append(bytes.Clone(data), 0), expected); err == nil {
						t.Fatal("accepted trailing data")
					}
					if _, err := readProof(data, expected+1); err == nil {
						t.Fatal("accepted wrong commitment count")
					}
				})
			}
			for _, count := range []uint32{1_048_576, 0xffff_ffff} {
				forged := bytes.Clone(compressed)
				binary.BigEndian.PutUint32(forged[128:132], count)
				if _, err := readProof(forged, expected); err == nil {
					t.Fatal("accepted forged commitment count")
				}
				// Even when a caller supplies a matching expected count, bytes
				// must exist before allocating the vector.
				if _, err := readProof(forged[:132], int(count)); err == nil {
					t.Fatal("allocated absent commitments")
				}
			}
			// Layout checks must not bypass gnark's curve checks.
			invalid := bytes.Clone(compressed)
			for i := 0; i < 32; i++ {
				invalid[i] = 0xff
			}
			if _, err := readProof(invalid, expected); err == nil {
				t.Fatal("accepted invalid curve point")
			}
		})
	}
}
