package main

import (
	"errors"
	"fmt"
	"github.com/consensys/gnark-crypto/ecc"
	curve "github.com/consensys/gnark-crypto/ecc/bn254"
	"github.com/consensys/gnark-crypto/ecc/bn254/fr"
	"github.com/consensys/gnark-crypto/ecc/bn254/fr/fft"
	"github.com/consensys/gnark/backend/groth16"
	native "github.com/consensys/gnark/backend/groth16/bn254"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/r1cs"
	"math/big"
	"testing"
)

// Exercise accelerated proof orchestration independently of the browser ABI.
// This reference uses gnark arithmetic; browser tests exercise the Rust kernel.
type referenceKernel struct {
	pk              *native.ProvingKey
	closed          int
	commitmentCalls int
	fail            bool
}

func (k *referenceKernel) close() { k.closed++ }
func (k *referenceKernel) parts(sa, sb, sk, a, b, c []fr.Element) (kernelParts, error) {
	var result kernelParts
	if k.fail {
		return result, errors.New("kernel failure")
	}
	domain := &k.pk.Domain
	pad := make([]fr.Element, int(domain.Cardinality)-len(a))
	a = append(a, pad...)
	b = append(b, pad...)
	c = append(c, pad...)
	for _, v := range [][]fr.Element{a, b, c} {
		domain.FFTInverse(v, fft.DIF)
		domain.FFT(v, fft.DIT, fft.OnCoset())
	}
	var den, one fr.Element
	one.SetOne()
	den.Exp(domain.FrMultiplicativeGen, big.NewInt(int64(domain.Cardinality))).Sub(&den, &one).Inverse(&den)
	for i := range a {
		a[i].Mul(&a[i], &b[i]).Sub(&a[i], &c[i]).Mul(&a[i], &den)
	}
	domain.FFTInverse(a, fft.DIF, fft.OnCoset())
	conf := ecc.MultiExpConfig{}
	if _, err := result.a.MultiExp(k.pk.G1.A, sa, conf); err != nil {
		return result, err
	}
	if _, err := result.b.MultiExp(k.pk.G1.B, sb, conf); err != nil {
		return result, err
	}
	if _, err := result.k.MultiExp(k.pk.G1.K, sk, conf); err != nil {
		return result, err
	}
	if _, err := result.z.MultiExp(k.pk.G1.Z, a[:len(a)-1], conf); err != nil {
		return result, err
	}
	if _, err := result.b2.MultiExp(k.pk.G2.B, sb, conf); err != nil {
		return result, err
	}
	return result, nil
}

type nestedCommitments struct {
	X, Z frontend.Variable
	Y    frontend.Variable `gnark:",public"`
}

func (c *nestedCommitments) Define(api frontend.API) error {
	committer := api.Compiler().(frontend.Committer)
	first, err := committer.Commit(c.X)
	if err != nil {
		return err
	}
	second, err := committer.Commit(c.Z, first, c.Y)
	if err != nil {
		return err
	}
	api.AssertIsDifferent(second, 0)
	api.AssertIsEqual(api.Add(api.Mul(c.X, c.X), api.Mul(c.Z, c.Z)), c.Y)
	return nil
}

func TestAcceleratedProofOrchestration(t *testing.T) {
	for _, commitments := range []int{0, 1, 2, 3} {
		t.Run(fmt.Sprint(commitments), func(t *testing.T) {
			var circuit, key, verifying []byte
			inputs := []string{`{"X":"0","Y":"5"}`, `{"X":"3","Y":"35"}`}
			if commitments == 0 {
				circuit = vector(t, "r1cs")
				key = vector(t, "pk")
				verifying = vector(t, "vk")
			} else {
				var definition frontend.Circuit = &committedCircuit{}
				inputs = []string{`{"X":"0","Y":"0"}`, `{"X":"3","Y":"9"}`}
				if commitments == 2 {
					definition = &nestedCommitments{}
					inputs = []string{`{"X":"0","Z":"0","Y":"0"}`, `{"X":"3","Z":"4","Y":"25"}`}
				}
				if commitments == 3 {
					definition = &manyCommittedValues{}
					y := big.NewInt(3)
					for i := 0; i < 512; i++ {
						y.Mul(y, y).Mod(y, ecc.BN254.ScalarField())
					}
					inputs = []string{`{"X":"0","Y":"0"}`, `{"X":"3","Y":"` + y.String() + `"}`}
				}
				cs, err := frontend.Compile(ecc.BN254.ScalarField(), r1cs.NewBuilder, definition)
				if err != nil {
					t.Fatal(err)
				}
				pk, vk, err := groth16.Setup(cs)
				if err != nil {
					t.Fatal(err)
				}
				circuit = serialized(t, cs)
				key = serialized(t, pk)
				verifying = serialized(t, vk)
			}
			prepared, err := prepareCircuit(circuit, key, verifying)
			if err != nil {
				t.Fatal(err)
			}
			kernel := &referenceKernel{pk: prepared.pk.(*native.ProvingKey)}
			prepared.kernel = kernel
			for _, input := range inputs {
				kernel.fail = true
				if _, err := prepared.prove(input); err == nil {
					t.Fatal("ignored arithmetic kernel failure")
				}
				kernel.fail = false
				result, err := prepared.prove(input)
				if err != nil {
					t.Fatal(err)
				}
				if valid, err := verify(circuit, verifying, result); err != nil || !valid {
					t.Fatalf("native verification valid=%v: %v", valid, err)
				}
			}
			if commitments == 3 && kernel.commitmentCalls < 4 {
				t.Fatal("large commitments did not use the kernel")
			}
			prepared.close()
			prepared.close()
			if kernel.closed != 1 {
				t.Fatal("kernel was not released exactly once")
			}
		})
	}
}

func (k *referenceKernel) commitment(index int, knowledge bool, values []fr.Element) (curve.G1Affine, error) {
	k.commitmentCalls++
	if k.fail {
		return curve.G1Affine{}, errors.New("kernel failure")
	}
	if knowledge {
		return k.pk.CommitmentKeys[index].ProveKnowledge(values)
	}
	return k.pk.CommitmentKeys[index].Commit(values)
}

type manyCommittedValues struct {
	X frontend.Variable
	Y frontend.Variable `gnark:",public"`
}

func (c *manyCommittedValues) Define(api frontend.API) error {
	v := c.X
	values := []frontend.Variable{v}
	for i := 0; i < 512; i++ {
		v = api.Mul(v, v)
		values = append(values, v)
	}
	commitment, err := api.Compiler().(frontend.Committer).Commit(values...)
	if err != nil {
		return err
	}
	api.AssertIsDifferent(commitment, 0)
	api.AssertIsEqual(v, c.Y)
	return nil
}
