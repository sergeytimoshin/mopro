package main

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark/backend/groth16"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/r1cs"
)

type binaryCircuit struct{ X frontend.Variable }

func (c *binaryCircuit) Define(api frontend.API) error { api.ToBinary(c.X, 8); return nil }

type largerCircuit struct {
	X frontend.Variable
	Y frontend.Variable `gnark:",public"`
}

func (c *largerCircuit) Define(api frontend.API) error {
	v := c.X
	for i := 0; i < 100; i++ {
		v = api.Mul(v, c.X)
	}
	api.AssertIsEqual(v, c.Y)
	return nil
}

// Build the shipping module separately. Building a Go test binary would mask
// missing hint imports because the test's frontend registers them itself.
func TestProductionWasm(t *testing.T) {
	dir := t.TempDir()
	wasm := filepath.Join(dir, "gnark.wasm")
	build := exec.Command("go", "build", "-mod=readonly", "-o", wasm, ".")
	build.Env = append(os.Environ(), "GOOS=js", "GOARCH=wasm", "CGO_ENABLED=0")
	if output, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build production Wasm: %v\n%s", err, output)
	}
	goroot, err := exec.Command("go", "env", "GOROOT").Output()
	if err != nil {
		t.Fatal(err)
	}
	shim := filepath.Join(strings.TrimSpace(string(goroot)), "lib", "wasm", "wasm_exec.js")
	fixtures := make(map[string]map[string]string)
	for name, definition := range map[string]frontend.Circuit{"binary": &binaryCircuit{}, "larger": &largerCircuit{}} {
		cs, err := frontend.Compile(ecc.BN254.ScalarField(), r1cs.NewBuilder, definition)
		if err != nil {
			t.Fatal(err)
		}
		pk, vk, err := groth16.Setup(cs)
		if err != nil {
			t.Fatal(err)
		}
		fixtures[name] = map[string]string{"r1cs": hex.EncodeToString(serialized(t, cs)), "pk": hex.EncodeToString(serialized(t, pk)), "vk": hex.EncodeToString(serialized(t, vk))}
	}
	fixtures["cubic"] = map[string]string{"r1cs": hex.EncodeToString(vector(t, "r1cs")), "pk": hex.EncodeToString(vector(t, "pk")), "vk": hex.EncodeToString(vector(t, "vk"))}
	encoded, err := json.Marshal(fixtures)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "fixtures.json")
	if err := os.WriteFile(path, encoded, 0600); err != nil {
		t.Fatal(err)
	}
	command := exec.Command("node", "test/production-wasm.mjs", wasm, shim, path)
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("production Wasm regression: %v\n%s", err, output)
	}
}
