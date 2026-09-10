// Generate fresh benchmark keys and witnesses; never use this setup in production.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"github.com/consensys/gnark-crypto/ecc"
	nativemimc "github.com/consensys/gnark-crypto/ecc/bn254/fr/mimc"
	"github.com/consensys/gnark/backend/groth16"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/r1cs"
	circuitmimc "github.com/consensys/gnark/std/hash/mimc"
	"io"
	"math/big"
	"os"
	"path/filepath"
)

type circuit struct {
	X           frontend.Variable
	Y           frontend.Variable `gnark:",public"`
	Rounds      int               `gnark:"-"`
	Commitments int               `gnark:"-"`
	CommitAll   bool              `gnark:"-"`
	MiMC        bool              `gnark:"-"`
}

func (c *circuit) Define(api frontend.API) error {
	value := c.X
	var first frontend.Variable
	if c.Commitments > 0 && !c.CommitAll {
		var err error
		first, err = api.Compiler().(frontend.Committer).Commit(c.X)
		if err != nil {
			return err
		}
		api.AssertIsDifferent(first, 0)
	}
	if c.MiMC {
		h, err := circuitmimc.NewMiMC(api)
		if err != nil {
			return err
		}
		for i := 0; i < c.Rounds; i++ {
			h.Write(c.X)
		}
		value = h.Sum()
	} else {
		committed := []frontend.Variable{c.X}
		for i := 0; i < c.Rounds; i++ {
			value = api.Mul(value, value)
			if c.CommitAll {
				committed = append(committed, value)
			}
			if c.CommitAll && i == c.Rounds/2 {
				var err error
				first, err = api.Compiler().(frontend.Committer).Commit(committed...)
				if err != nil {
					return err
				}
				api.AssertIsDifferent(first, 0)
				committed = nil
			}
			if !c.CommitAll && c.Commitments > 1 && i == c.Rounds/2 {
				second, err := api.Compiler().(frontend.Committer).Commit(value, first, c.Y)
				if err != nil {
					return err
				}
				api.AssertIsDifferent(second, 0)
			}
		}
		if c.CommitAll && c.Commitments > 1 {
			committed = append(committed, first, c.Y)
			second, err := api.Compiler().(frontend.Committer).Commit(committed...)
			if err != nil {
				return err
			}
			api.AssertIsDifferent(second, 0)
		}
	}
	api.AssertIsEqual(value, c.Y)
	return nil
}
func main() {
	output := flag.String("out", "../web/assets/gnark-bench", "fixture directory")
	rounds := flag.Int("rounds", 16384, "number of squarings (or MiMC input words)")
	commitments := flag.Int("commitments", 0, "number of commitments: 0, 1, or 2")
	useMiMC := flag.Bool("mimc", false, "use a MiMC hash circuit")
	commitAll := flag.Bool("commit-all", false, "commit to intermediate square values")
	uncompressed := flag.Bool("uncompressed", false, "write larger keys that avoid browser point decompression")
	flag.Parse()
	if *rounds < 2 || *commitments < 0 || *commitments > 2 || (*useMiMC && *commitments > 1) || (*commitAll && (*useMiMC || *commitments == 0)) {
		panic("invalid fixture options")
	}
	definition := &circuit{Rounds: *rounds, Commitments: *commitments, MiMC: *useMiMC, CommitAll: *commitAll}
	cs, err := frontend.Compile(ecc.BN254.ScalarField(), r1cs.NewBuilder, definition)
	if err != nil {
		panic(err)
	}
	pk, vk, err := groth16.Setup(cs)
	if err != nil {
		panic(err)
	}
	if err := os.MkdirAll(*output, 0755); err != nil {
		panic(err)
	}
	for name, value := range map[string]io.WriterTo{"circuit.r1cs": cs, "circuit.pk": pk, "circuit.vk": vk} {
		f, err := os.Create(filepath.Join(*output, name))
		if err != nil {
			panic(err)
		}
		var writeErr error
		if raw, ok := value.(interface {
			WriteRawTo(io.Writer) (int64, error)
		}); *uncompressed && ok {
			_, writeErr = raw.WriteRawTo(f)
		} else {
			_, writeErr = value.WriteTo(f)
		}
		closeErr := f.Close()
		if writeErr != nil {
			panic(writeErr)
		}
		if closeErr != nil {
			panic(closeErr)
		}
	}
	inputs := []map[string]string{}
	for _, x := range []int64{3, 4, 0} {
		y := big.NewInt(x)
		if *useMiMC {
			h := nativemimc.NewMiMC()
			var encoded [32]byte
			y.FillBytes(encoded[:])
			for i := 0; i < *rounds; i++ {
				if _, err := h.Write(encoded[:]); err != nil {
					panic(err)
				}
			}
			y.SetBytes(h.Sum(nil))
		} else {
			for i := 0; i < *rounds; i++ {
				y.Mul(y, y).Mod(y, ecc.BN254.ScalarField())
			}
		}
		input := map[string]string{"X": fmt.Sprint(x), "Y": y.String()}
		w, err := frontend.NewWitness(&circuit{X: x, Y: y}, ecc.BN254.ScalarField())
		if err != nil {
			panic(err)
		}
		proof, err := groth16.Prove(cs, pk, w)
		if err != nil {
			panic(err)
		}
		public, err := w.Public()
		if err != nil {
			panic(err)
		}
		if err := groth16.Verify(proof, vk, public); err != nil {
			panic(err)
		}
		inputs = append(inputs, input)
	}
	data, err := json.MarshalIndent(map[string]any{"constraints": cs.GetNbConstraints(), "rounds": *rounds, "commitments": *commitments, "mimc": *useMiMC, "commitAll": *commitAll, "uncompressed": *uncompressed, "inputs": inputs}, "", "  ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(filepath.Join(*output, "fixture.json"), data, 0644); err != nil {
		panic(err)
	}
	fmt.Printf("Generated %d constraints at %s\n", cs.GetNbConstraints(), *output)
}
