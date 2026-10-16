package main

import (
	"bytes"
	"encoding/hex"
	"io"
	"math/big"
	"os"
	"testing"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark/backend/groth16"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/r1cs"
)

func vector(t *testing.T, extension string) []byte {
	t.Helper()
	data, err := os.ReadFile("../test-vectors/gnark/cubic_circuit." + extension)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

type cubicAssignment struct {
	X frontend.Variable
	Y frontend.Variable `gnark:",public"`
}

func (*cubicAssignment) Define(frontend.API) error { return nil }

func TestProveVerifyAndNativeCompatibility(t *testing.T) {
	r1cs, pk, vk := vector(t, "r1cs"), vector(t, "pk"), vector(t, "vk")
	result, err := prove(r1cs, pk, `{"X":"3","Y":"35"}`)
	if err != nil {
		t.Fatal(err)
	}
	if valid, err := verify(r1cs, vk, result); err != nil || !valid {
		t.Fatalf("round trip: valid=%v, err=%v", valid, err)
	}

	if result.Execution != (proofExecution{Arithmetic: "go", Solver: "go"}) {
		t.Fatalf("incorrect default execution: %+v", result.Execution)
	}

	// Build the witness independently through gnark's native frontend API.
	assignment, err := frontend.NewWitness(&cubicAssignment{X: "3", Y: "35"}, ecc.BN254.ScalarField())
	if err != nil {
		t.Fatal(err)
	}
	public, err := assignment.Public()
	if err != nil {
		t.Fatal(err)
	}
	publicBytes, err := public.MarshalBinary()
	if err != nil || result.PublicInputs != hex.EncodeToString(publicBytes) {
		t.Fatalf("public witness differs from native serialization: %v", err)
	}
	cs, err := readCircuit(r1cs)
	if err != nil {
		t.Fatal(err)
	}
	nativeKey := groth16.NewProvingKey(ecc.BN254)
	if _, err := nativeKey.ReadFrom(bytes.NewReader(pk)); err != nil {
		t.Fatal(err)
	}
	nativeProof, err := groth16.Prove(cs, nativeKey, assignment)
	if err != nil {
		t.Fatal(err)
	}
	var proof bytes.Buffer
	if _, err := nativeProof.WriteTo(&proof); err != nil {
		t.Fatal(err)
	}
	result.Proof = hex.EncodeToString(proof.Bytes())
	if valid, err := verify(r1cs, vk, result); err != nil || !valid {
		t.Fatalf("native proof: valid=%v, err=%v", valid, err)
	}
	publicBytes[len(publicBytes)-1] ^= 1
	result.PublicInputs = hex.EncodeToString(publicBytes)
	if valid, err := verify(r1cs, vk, result); err != nil || valid {
		t.Fatalf("altered public input: valid=%v, err=%v", valid, err)
	}
	result.Proof = "not hex"
	if _, err := verify(r1cs, vk, result); err == nil {
		t.Fatal("accepted malformed proof")
	}
}

func TestRejectInvalidInputs(t *testing.T) {
	r1cs, pk := vector(t, "r1cs"), vector(t, "pk")
	for _, input := range []string{
		`{"X":"3","Y":"36"}`, // unsatisfied circuit
		`{"Y":"35"}`,         // missing secret
		`{"X":3,"Y":"35"}`,   // reject lossy JavaScript numbers
		`{"X":"bad","Y":"35"}`,
		`{"X":null,"Y":"35"}`,
		`null`,
		`{`,
	} {
		t.Run(input, func(t *testing.T) {
			if _, err := prove(r1cs, pk, input); err == nil {
				t.Fatal("accepted invalid witness")
			}
		})
	}
	if _, err := prove(nil, pk, `{"X":"3","Y":"35"}`); err == nil {
		t.Fatal("accepted empty R1CS")
	}
	if _, err := prove(r1cs, nil, `{"X":"3","Y":"35"}`); err == nil {
		t.Fatal("accepted empty proving key")
	}
}

func TestLargeFieldValues(t *testing.T) {
	x, _ := new(big.Int).SetString("9007199254740993", 10)
	y := new(big.Int).Exp(x, big.NewInt(3), ecc.BN254.ScalarField())
	y.Add(y, x).Add(y, big.NewInt(5)).Mod(y, ecc.BN254.ScalarField())
	input := `{"X":"` + x.String() + `","Y":"` + y.String() + `"}`
	result, err := prove(vector(t, "r1cs"), vector(t, "pk"), input)
	if err != nil {
		t.Fatal(err)
	}
	if valid, err := verify(vector(t, "r1cs"), vector(t, "vk"), result); err != nil || !valid {
		t.Fatalf("large field values: valid=%v, err=%v", valid, err)
	}
}

type committedCircuit struct {
	X frontend.Variable
	Y frontend.Variable `gnark:",public"`
}

func (c *committedCircuit) Define(api frontend.API) error {
	commitment, err := api.Compiler().(frontend.Committer).Commit(c.X)
	if err != nil {
		return err
	}
	api.AssertIsDifferent(commitment, 0)
	api.AssertIsEqual(api.Mul(c.X, c.X), c.Y)
	return nil
}

func serialized(t *testing.T, value io.WriterTo) []byte {
	t.Helper()
	var buf bytes.Buffer
	if _, err := value.WriteTo(&buf); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

func TestCircuitWithCommitment(t *testing.T) {
	cs, err := frontend.Compile(ecc.BN254.ScalarField(), r1cs.NewBuilder, &committedCircuit{})
	if err != nil {
		t.Fatal(err)
	}
	pk, vk, err := groth16.Setup(cs)
	if err != nil {
		t.Fatal(err)
	}
	circuit := serialized(t, cs)
	result, err := prove(circuit, serialized(t, pk), `{"X":"3","Y":"9"}`)
	if err != nil {
		t.Fatal(err)
	}
	if valid, err := verify(circuit, serialized(t, vk), result); err != nil || !valid {
		t.Fatalf("commitment circuit: valid=%v, err=%v", valid, err)
	}
	prepared, err := prepareCircuit(circuit, serialized(t, pk), serialized(t, vk))
	if err != nil {
		t.Fatal(err)
	}
	for _, input := range []string{`{"X":"3","Y":"9"}`, `{"X":"4","Y":"16"}`} {
		result, err := prepared.prove(input)
		if err != nil {
			t.Fatal(err)
		}
		if valid, err := prepared.verify(result); err != nil || !valid {
			t.Fatalf("prepared commitment circuit: valid=%v, err=%v", valid, err)
		}
	}
}

func TestPreparedCircuitReusesKeysWithFreshWitnesses(t *testing.T) {
	r1cs, pk, vk := vector(t, "r1cs"), vector(t, "pk"), vector(t, "vk")
	circuit, err := prepareCircuit(r1cs, pk, vk)
	if err != nil {
		t.Fatal(err)
	}
	// Retain decoded objects, not views into the caller's source buffers.
	clear(r1cs)
	clear(pk)
	clear(vk)
	var previous string
	for _, input := range []string{`{"X":"1","Y":"7"}`, `{"X":"2","Y":"15"}`, `{"X":"3","Y":"35"}`} {
		if _, err := circuit.prove(`{"X":"3","Y":"36"}`); err == nil {
			t.Fatal("accepted unsatisfied witness")
		}
		result, err := circuit.prove(input)
		if err != nil {
			t.Fatal(err)
		}
		if result.PublicInputs == previous {
			t.Fatal("reused the previous witness")
		}
		previous = result.PublicInputs
		if valid, err := circuit.verify(result); err != nil || !valid {
			t.Fatalf("prepared circuit: valid=%v, err=%v", valid, err)
		}
		if valid, err := verify(vector(t, "r1cs"), vector(t, "vk"), result); err != nil || !valid {
			t.Fatalf("prepared proof through one-shot API: valid=%v, err=%v", valid, err)
		}
	}
	legacy, err := prove(vector(t, "r1cs"), vector(t, "pk"), `{"X":"3","Y":"35"}`)
	if err != nil {
		t.Fatal(err)
	}
	if valid, err := circuit.verify(legacy); err != nil || !valid {
		t.Fatalf("one-shot proof through prepared API: valid=%v, err=%v", valid, err)
	}
}

func TestPreparedCircuitOptionalAndInvalidKeys(t *testing.T) {
	r1cs, pk, vk := vector(t, "r1cs"), vector(t, "pk"), vector(t, "vk")
	prover, err := prepareCircuit(r1cs, pk, nil)
	if err != nil {
		t.Fatal(err)
	}
	result, err := prover.prove(`{"X":"3","Y":"35"}`)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := prover.verify(result); err == nil {
		t.Fatal("verified without a verifying key")
	}
	verifier, err := prepareCircuit(r1cs, nil, vk)
	if err != nil {
		t.Fatal(err)
	}
	if valid, err := verifier.verify(result); err != nil || !valid {
		t.Fatalf("verifier-only circuit: valid=%v, err=%v", valid, err)
	}
	if _, err := verifier.prove(`{"X":"3","Y":"35"}`); err == nil {
		t.Fatal("proved without a proving key")
	}
	for _, keys := range []struct{ pk, vk []byte }{
		{nil, nil}, {[]byte{}, vk}, {pk, []byte{}},
	} {
		if _, err := prepareCircuit(r1cs, keys.pk, keys.vk); err == nil {
			t.Fatal("prepared with missing or malformed keys")
		}
	}
}
