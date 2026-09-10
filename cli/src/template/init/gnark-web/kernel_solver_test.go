package main

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"math/rand"
	"os"
	"path/filepath"
	"testing"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark-crypto/ecc/bn254/fr"
	"github.com/consensys/gnark/backend"
	native "github.com/consensys/gnark/backend/groth16/bn254"
	"github.com/consensys/gnark/backend/witness"
	"github.com/consensys/gnark/constraint"
	cs "github.com/consensys/gnark/constraint/bn254"
	"github.com/consensys/gnark/constraint/solver"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/r1cs"
)

func solverWitness(t *testing.T, system *cs.R1CS, values []fr.Element) witness.Witness {
	t.Helper()
	w, err := witness.New(ecc.BN254.ScalarField())
	if err != nil {
		t.Fatal(err)
	}
	ch := make(chan any, len(values))
	for _, value := range values {
		ch <- value
	}
	close(ch)
	if err := w.Fill(len(system.Public)-1, len(system.Secret), ch); err != nil {
		t.Fatal(err)
	}
	return w
}

// Low-level R1Cs deliberately exercise shapes the frontend normally simplifies:
// any unknown side, non-unit unknown coefficients, zero divisors, long linear
// expressions and constraints whose dependency order differs from their row ID.
func solverFixture(seed int64) *cs.R1CS {
	system := cs.NewR1CS(256)
	system.AddPublicVariable("1")
	system.AddPublicVariable("Y")
	system.AddSecretVariable("X")
	system.AddSecretVariable("Z")
	blueprint := system.AddBlueprint(&constraint.BlueprintGenericR1C{})
	rng := rand.New(rand.NewSource(seed))
	coefficients := []int64{0, 1, 2, -1, -2, 7, -17, 123456789}
	term := func(coefficient int64, wire int) constraint.Term {
		return system.MakeTerm(system.FromInterface(coefficient), wire)
	}
	one := constraint.LinearExpression{term(1, 0)}
	for i := 0; i < 96; i++ {
		nKnown := 4 + i
		var expressions [3]constraint.LinearExpression
		for side := range expressions {
			for j := 0; j < 1+rng.Intn(5); j++ {
				expressions[side] = append(expressions[side], term(coefficients[rng.Intn(len(coefficients))], rng.Intn(nKnown)))
			}
		}
		side := i % 3
		if side < 2 {
			// A witness-dependent divisor, including zero with zero output.
			expressions[1-side] = constraint.LinearExpression{term(5, 3)}
			expressions[2] = constraint.LinearExpression{term(7, 3)}
			if i%7 == 0 {
				expressions[1-side] = nil
				expressions[2] = nil
			}
		}
		wire := system.AddInternalVariable()
		expressions[side] = append(expressions[side], term(coefficients[1+rng.Intn(len(coefficients)-1)], wire))
		r := constraint.R1C{L: expressions[0], R: expressions[1], O: expressions[2]}
		system.AddR1C(r, blueprint)
		// This second row is an assertion, with no unknown term.
		system.AddR1C(r, blueprint)
	}
	system.AddR1C(constraint.R1C{L: constraint.LinearExpression{term(1, 2)}, R: one, O: constraint.LinearExpression{term(1, 1)}}, blueprint)
	return system
}

// Set MOPRO_GNARK_SOLVER_FIXTURES to export gnark's exact field results for
// the Rust differential test. CI runs both against the same temporary directory.
func TestKernelSolverFixtures(t *testing.T) {
	for seed := int64(1); seed <= 4; seed++ {
		system := solverFixture(seed)
		n := len(system.Public) + len(system.Secret) + system.NbInternalVariables
		a, b := make([]bool, n), make([]bool, n)
		for i := range a {
			a[i] = i%3 == 0
			b[i] = i%5 == 0
		}
		program := kernelSolverProgram(system, a, b)
		if program == nil {
			t.Fatal("ordinary R1CS did not produce a solver plan")
		}
		for sample, x := range []int64{0, 1, -1, 987654321} {
			values := make([]fr.Element, 3)
			values[0].SetInt64(x)
			values[1].SetInt64(x)
			values[2].SetInt64(x + seed)
			w := solverWitness(t, system, values)
			result, err := system.Solve(w)
			if err != nil {
				t.Fatal(err)
			}
			solution := result.(*cs.R1CSSolution)
			invalid := append([]fr.Element(nil), values...)
			one := fr.One()
			invalid[0].Add(&invalid[0], &one)
			if _, err := system.Solve(solverWitness(t, system, invalid)); err == nil {
				t.Fatal("native solver accepted invalid assertion")
			}
			if dir := os.Getenv("MOPRO_GNARK_SOLVER_FIXTURES"); dir != "" {
				if err := os.MkdirAll(dir, 0755); err != nil {
					t.Fatal(err)
				}
				var encoded, plan bytes.Buffer
				if err := binary.Write(&plan, binary.LittleEndian, program); err != nil {
					t.Fatal(err)
				}
				for _, data := range [][]byte{plan.Bytes(), encodeKernelFields(system.Coefficients), encodeKernelFields(values), encodeKernelFields(solution.W), encodeKernelFields(solution.A), encodeKernelFields(solution.B), encodeKernelFields(solution.C), encodeKernelFields(invalid)} {
					if err := binary.Write(&encoded, binary.LittleEndian, uint32(len(data))); err != nil {
						t.Fatal(err)
					}
					encoded.Write(data)
				}
				if err := os.WriteFile(filepath.Join(dir, fmt.Sprintf("%d-%d.case", seed, sample)), encoded.Bytes(), 0644); err != nil {
					t.Fatal(err)
				}
			}
		}
	}
}

type solverLogCircuit struct{ X frontend.Variable }

func (c *solverLogCircuit) Define(api frontend.API) error {
	api.Println(c.X)
	api.AssertIsEqual(api.Mul(c.X, c.X), 9)
	return nil
}

type solverHintCircuit struct{ X frontend.Variable }

func (c *solverHintCircuit) Define(api frontend.API) error { api.ToBinary(c.X, 8); return nil }

// A custom blueprint which happens to have the same encoding must still fall back.
type customR1C struct{ constraint.BlueprintGenericR1C }

func TestKernelSolverFallback(t *testing.T) {
	for _, definition := range []frontend.Circuit{&solverLogCircuit{}, &solverHintCircuit{}, &committedCircuit{}} {
		compiled, err := frontend.Compile(ecc.BN254.ScalarField(), r1cs.NewBuilder, definition)
		if err != nil {
			t.Fatal(err)
		}
		system := compiled.(*cs.R1CS)
		n := len(system.Public) + len(system.Secret) + system.NbInternalVariables
		if kernelSolverProgram(system, make([]bool, n), make([]bool, n)) != nil {
			t.Fatalf("%T should retain Go solving", definition)
		}
	}
	system := solverFixture(1)
	n := len(system.Public) + len(system.Secret) + system.NbInternalVariables
	system.Blueprints[system.Instructions[0].BlueprintID] = &customR1C{}
	if kernelSolverProgram(system, make([]bool, n), make([]bool, n)) != nil {
		t.Fatal("custom blueprint should retain Go solving")
	}
}

type referenceWitnessKernel struct {
	referenceKernel
	system  *cs.R1CS
	t       *testing.T
	calls   int
	enabled bool
}

func (k *referenceWitnessKernel) solveParts(values []fr.Element) (kernelParts, bool, error) {
	k.calls++
	if !k.enabled {
		return kernelParts{}, false, nil
	}
	result, err := k.system.Solve(solverWitness(k.t, k.system, values))
	if err != nil {
		return kernelParts{}, true, err
	}
	s := result.(*cs.R1CSSolution)
	filter := func(infinity []bool) []fr.Element {
		var values []fr.Element
		for i, infinite := range infinity {
			if !infinite {
				values = append(values, s.W[i])
			}
		}
		return values
	}
	parts, err := k.parts(filter(k.pk.InfinityA), filter(k.pk.InfinityB), s.W[len(k.system.Public):], s.A, s.B, s.C)
	return parts, true, err
}
func TestAcceleratedWitnessOrchestration(t *testing.T) {
	c, err := prepareCircuit(vector(t, "r1cs"), vector(t, "pk"), vector(t, "vk"))
	if err != nil {
		t.Fatal(err)
	}
	k := &referenceWitnessKernel{referenceKernel: referenceKernel{pk: c.pk.(*native.ProvingKey)}, system: c.cs, t: t, enabled: true}
	c.kernel = k
	for _, input := range []string{`{"X":"0","Y":"5"}`, `{"X":"3","Y":"35"}`} {
		p, err := c.prove(input)
		if err != nil {
			t.Fatal(err)
		}
		if p.Execution != (proofExecution{Arithmetic: "rust", Solver: "rust"}) {
			t.Fatalf("incorrect solver execution: %+v", p.Execution)
		}
		if ok, err := c.verify(p); err != nil || !ok {
			t.Fatalf("invalid proof: %v", err)
		}
	}
	if k.calls != 2 {
		t.Fatal("witness kernel was not used")
	}
	if _, err := c.prove(`{"X":"3","Y":"1"}`); err == nil {
		t.Fatal("ignored solver failure")
	}
	k.fail = true
	if _, err := c.prove(`{"X":"3","Y":"35"}`); err == nil {
		t.Fatal("ignored arithmetic failure")
	}
	k.fail = false
	// An unsupported plan uses Go solving with the same arithmetic kernel.
	k.enabled = false
	fallback, err := c.prove(`{"X":"3","Y":"35"}`)
	if err != nil {
		t.Fatal(err)
	}
	if fallback.Execution != (proofExecution{Arithmetic: "rust", Solver: "go"}) {
		t.Fatalf("incorrect fallback execution: %+v", fallback.Execution)
	}
	k.enabled = true
	calls := k.calls
	w, err := buildWitness(`{"X":"3","Y":"35"}`, c.cs)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := proveAccelerated(c.cs, k.pk, k, w, &proofExecution{}, backend.WithSolverOptions(solver.WithNbTasks(1))); err != nil {
		t.Fatal(err)
	}
	if k.calls != calls {
		t.Fatal("custom solver options bypassed")
	}
}
