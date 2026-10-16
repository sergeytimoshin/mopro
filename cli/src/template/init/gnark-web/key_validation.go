package main

import (
	"fmt"
	"slices"

	"github.com/consensys/gnark-crypto/ecc"
	native "github.com/consensys/gnark/backend/groth16/bn254"
	"github.com/consensys/gnark/constraint"
)

// Check the dimensions used by gnark before its prover starts goroutines:
// a panic in those goroutines cannot be recovered by the JS callback guard.
// This checks structural compatibility, not which setup produced the keys.
func (c *preparedCircuit) validateKeys() error {
	commitments, ok := c.cs.CommitmentInfo.(constraint.Groth16Commitments)
	if !ok || c.cs.Type != constraint.SystemR1CS {
		return fmt.Errorf("expected Groth16 R1CS commitments")
	}
	public := c.cs.GetNbPublicVariables()
	wires := public + c.cs.GetNbSecretVariables() + c.cs.NbInternalVariables
	private := wires - public - len(commitments)
	for _, commitment := range commitments {
		private -= len(commitment.PrivateCommitted)
	}
	if private < 0 {
		return fmt.Errorf("invalid circuit commitment dimensions")
	}
	if c.pk != nil {
		pk := c.pk.(*native.ProvingKey)
		domain := ecc.NextPowerOfTwo(uint64(c.cs.GetNbConstraints()))
		if pk.Domain.Cardinality != domain || uint64(len(pk.G1.Z)) != domain-1 ||
			len(pk.G1.K) != private || len(pk.G2.B) != len(pk.G1.B) ||
			!validInfinity(pk.InfinityA, pk.NbInfinityA, len(pk.G1.A), wires) ||
			!validInfinity(pk.InfinityB, pk.NbInfinityB, len(pk.G1.B), wires) ||
			len(pk.CommitmentKeys) != len(commitments) {
			return fmt.Errorf("proving key dimensions do not match the circuit")
		}
		for i, commitment := range commitments {
			key := &pk.CommitmentKeys[i]
			if len(key.Basis) != len(commitment.PrivateCommitted) || len(key.BasisExpSigma) != len(key.Basis) {
				return fmt.Errorf("proving key commitment %d does not match the circuit", i)
			}
		}
	}
	if c.vk != nil {
		vk := c.vk.(*native.VerifyingKey)
		if len(vk.G1.K) != public+len(commitments) || len(vk.CommitmentKeys) != len(commitments) || len(vk.PublicAndCommitmentCommitted) != len(commitments) {
			return fmt.Errorf("verifying key dimensions do not match the circuit")
		}
		// Translate earlier commitment wires into the public witness positions
		// used by the verifier, as gnark Setup does.
		positions := make(map[int]int, len(commitments))
		for i, commitment := range commitments {
			if commitment.NbPublicCommitted < 0 || commitment.NbPublicCommitted > len(commitment.PublicAndCommitmentCommitted) {
				return fmt.Errorf("invalid circuit commitment %d", i)
			}
			expected := slices.Clone(commitment.PublicAndCommitmentCommitted)
			for j, wire := range expected {
				if j < commitment.NbPublicCommitted {
					if wire < 1 || wire >= public {
						return fmt.Errorf("invalid public commitment wire")
					}
				} else {
					position, ok := positions[wire]
					if !ok {
						return fmt.Errorf("invalid earlier commitment wire")
					}
					expected[j] = position
				}
			}
			if !slices.Equal(vk.PublicAndCommitmentCommitted[i], expected) {
				return fmt.Errorf("verifying key commitment %d does not match the circuit", i)
			}
			positions[commitment.CommitmentIndex] = public + i
		}
	}
	return nil
}

func validInfinity(infinity []bool, declared uint64, points, wires int) bool {
	if len(infinity) != wires {
		return false
	}
	var count uint64
	for _, infinite := range infinity {
		if infinite {
			count++
		}
	}
	return count == declared && uint64(points)+count == uint64(wires)
}
