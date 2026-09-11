package main

import (
	"os"
	"testing"

	"github.com/consensys/gnark/logger"
)

// Native baseline for the same load-on-every-call and prepared code paths used
// by the browser. Run: go test -run '^$' -bench BenchmarkCubic -benchmem
func BenchmarkCubic(b *testing.B) {
	logger.Disable()
	read := func(extension string) []byte {
		data, err := os.ReadFile("../test-vectors/gnark/cubic_circuit." + extension)
		if err != nil {
			b.Fatal(err)
		}
		return data
	}
	r1cs, pk, vk := read("r1cs"), read("pk"), read("vk")
	input := `{"X":"3","Y":"35"}`
	prepared, err := prepareCircuit(r1cs, pk, vk)
	if err != nil {
		b.Fatal(err)
	}
	result, err := prepared.prove(input)
	if err != nil {
		b.Fatal(err)
	}
	b.Run("Prepare", func(b *testing.B) {
		for i := 0; i < b.N; i++ {
			if _, err := prepareCircuit(r1cs, pk, vk); err != nil {
				b.Fatal(err)
			}
		}
	})
	for _, name := range []string{"OneShot", "Prepared"} {
		b.Run(name+"/Prove", func(b *testing.B) {
			for i := 0; i < b.N; i++ {
				var err error
				if name == "Prepared" {
					_, err = prepared.prove(input)
				} else {
					_, err = prove(r1cs, pk, input)
				}
				if err != nil {
					b.Fatal(err)
				}
			}
		})
		b.Run(name+"/Verify", func(b *testing.B) {
			for i := 0; i < b.N; i++ {
				var valid bool
				var err error
				if name == "Prepared" {
					valid, err = prepared.verify(result)
				} else {
					valid, err = verify(r1cs, vk, result)
				}
				if err != nil || !valid {
					b.Fatalf("valid=%v, err=%v", valid, err)
				}
			}
		})
	}
}
