// Convert existing benchmark keys without running a new setup.
package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark/backend/groth16"
)

func main() {
	input := flag.String("in", "", "existing fixture directory")
	output := flag.String("out", "", "new fixture directory")
	flag.Parse()
	if *input == "" || *output == "" || *input == *output {
		panic("provide distinct input and output directories")
	}
	must(os.MkdirAll(*output, 0755))
	for _, name := range []string{"circuit.r1cs", "fixture.json", "circuit.pk", "circuit.vk"} {
		data, err := os.ReadFile(filepath.Join(*input, name))
		must(err)
		switch name {
		case "fixture.json":
			var meta map[string]any
			must(json.Unmarshal(data, &meta))
			meta["uncompressed"] = true
			data, err = json.MarshalIndent(meta, "", "  ")
			must(err)
		case "circuit.pk", "circuit.vk":
			var key interface {
				io.ReaderFrom
				io.WriterTo
				WriteRawTo(io.Writer) (int64, error)
			}
			if name == "circuit.pk" {
				key = groth16.NewProvingKey(ecc.BN254)
			} else {
				key = groth16.NewVerifyingKey(ecc.BN254)
			}
			_, err := key.ReadFrom(bytes.NewReader(data))
			must(err)
			var compressed, raw bytes.Buffer
			_, err = key.WriteTo(&compressed)
			must(err)
			if !bytes.Equal(data, compressed.Bytes()) {
				panic("input must be the compressed fixture")
			}
			_, err = key.WriteRawTo(&raw)
			must(err)
			// Validate with a fresh ordinary checked reader.
			if name == "circuit.pk" {
				key = groth16.NewProvingKey(ecc.BN254)
			} else {
				key = groth16.NewVerifyingKey(ecc.BN254)
			}
			_, err = key.ReadFrom(bytes.NewReader(raw.Bytes()))
			must(err)
			var roundtrip bytes.Buffer
			_, err = key.WriteTo(&roundtrip)
			must(err)
			if !bytes.Equal(data, roundtrip.Bytes()) {
				panic("key changed during conversion")
			}
			data = raw.Bytes()
		}
		must(os.WriteFile(filepath.Join(*output, name), data, 0644))
	}
	fmt.Println("Converted and round-trip checked", *output)
}
func must(err error) {
	if err != nil {
		panic(err)
	}
}
