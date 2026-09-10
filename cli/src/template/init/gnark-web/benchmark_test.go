package main

import (
	"encoding/json"
	"github.com/consensys/gnark/logger"
	"os"
	"path/filepath"
	"testing"
)

func benchmarkFixtureDir() string {
	if dir := os.Getenv("MOPRO_GNARK_BENCH_FIXTURES"); dir != "" {
		return dir
	}
	return "../web/assets/gnark-bench"
}
func TestBrowserBenchmarkProofs(t *testing.T) {
	report := os.Getenv("MOPRO_GNARK_BENCH_REPORT")
	if report == "" {
		t.Skip("set MOPRO_GNARK_BENCH_REPORT to cross-verify browser proofs")
	}
	data, err := os.ReadFile(report)
	if err != nil {
		t.Fatal(err)
	}
	var results struct {
		Proofs []proofResult `json:"proofs"`
	}
	if err := json.Unmarshal(data, &results); err != nil {
		t.Fatal(err)
	}
	if len(results.Proofs) < 9 {
		t.Fatal("browser benchmark did not produce all proofs")
	}
	read := func(name string) []byte {
		data, err := os.ReadFile(filepath.Join(benchmarkFixtureDir(), name))
		if err != nil {
			t.Fatal(err)
		}
		return data
	}
	verifier, err := prepareCircuit(read("circuit.r1cs"), nil, read("circuit.vk"))
	if err != nil {
		t.Fatal(err)
	}
	for i, proof := range results.Proofs {
		if valid, err := verifier.verify(proof); err != nil || !valid {
			t.Fatalf("proof %d: valid=%v, err=%v", i, valid, err)
		}
	}
}
func BenchmarkWebFixture(b *testing.B) {
	logger.Disable()
	read := func(name string) []byte {
		data, err := os.ReadFile(filepath.Join(benchmarkFixtureDir(), name))
		if err != nil {
			b.Fatalf("generate fixtures with go run ./cmd/benchmark first: %v", err)
		}
		return data
	}
	var fixture struct {
		Inputs []map[string]string `json:"inputs"`
	}
	if err := json.Unmarshal(read("fixture.json"), &fixture); err != nil {
		b.Fatal(err)
	}
	if len(fixture.Inputs) == 0 {
		b.Fatal("fixture has no witness")
	}
	input, err := json.Marshal(fixture.Inputs[0])
	if err != nil {
		b.Fatal(err)
	}
	c, err := prepareCircuit(read("circuit.r1cs"), read("circuit.pk"), read("circuit.vk"))
	if err != nil {
		b.Fatal(err)
	}
	proof, err := c.prove(string(input))
	if err != nil {
		b.Fatal(err)
	}
	if ok, err := c.verify(proof); err != nil || !ok {
		b.Fatalf("warmup invalid: %v", err)
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		if _, err := c.prove(string(input)); err != nil {
			b.Fatal(err)
		}
	}
}
