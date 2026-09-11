// Generate synthetic bridge vectors; all point/scalar arithmetic uses gnark.
package main

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	curve "github.com/consensys/gnark-crypto/ecc/bn254"
	"github.com/consensys/gnark-crypto/ecc/bn254/fr"
	"github.com/consensys/gnark-crypto/ecc/bn254/fr/fft"
	"math/big"
	"os"
	"path/filepath"
)

func encoded(value any) string {
	var b bytes.Buffer
	if err := binary.Write(&b, binary.LittleEndian, value); err != nil {
		panic(err)
	}
	return hex.EncodeToString(b.Bytes())
}
func main() {
	out := flag.String("out", "", "vector output directory")
	flag.Parse()
	if *out == "" {
		panic("missing out")
	}
	must(os.MkdirAll(*out, 0755))
	_, _, g1, g2 := curve.Generators()
	var p1 [37]curve.G1Affine
	var p2 [37]curve.G2Affine
	for i := 1; i < 37; i++ {
		p1[i].ScalarMultiplication(&g1, big.NewInt(int64(i)))
		p2[i].ScalarMultiplication(&g2, big.NewInt(int64(i)))
	}
	cases := []string{}
	for _, n := range []int{0, 1, 3, 31, 257, 1025, 4097} {
		size := uint64(2)
		for size < uint64(n+1) {
			size *= 2
		}
		d := fft.NewDomain(size)
		first := make([]curve.G1Affine, n)
		second := make([]curve.G2Affine, n)
		z := make([]curve.G1Affine, size-1)
		for i := range first {
			first[i] = p1[i%37]
			second[i] = p2[i%37]
		}
		for i := range z {
			z[i] = p1[(i+3)%37]
		}
		for _, power := range []int64{1, 3} {
			root := d.Generator
			root.Exp(root, big.NewInt(power))
			shift := fr.NewElement(5)
			scalarSets := []string{}
			evaluations := []string{}
			for round := 0; round < 3; round++ {
				s := make([]fr.Element, n)
				for i := range s {
					switch i % 4 {
					case 0:
						s[i].SetZero()
					case 1:
						s[i].SetOne()
					case 2:
						s[i].SetOne().Neg(&s[i])
					default:
						s[i].SetUint64(uint64(i + round + 11))
						s[i].Exp(s[i], big.NewInt(19))
					}
				}
				scalarSets = append(scalarSets, encoded(s))
				v := make([]fr.Element, size)
				for i := range v {
					v[i].SetUint64(uint64(i*i + round + 3))
				}
				evaluations = append(evaluations, encoded(v))
			}
			name := fmt.Sprintf("%d-root%d.json", n, power)
			value := map[string]any{"n": n, "size": size, "a": encoded(first), "b": encoded(first), "k": encoded(first), "z": encoded(z), "b2": encoded(second), "params": encoded([]fr.Element{root, shift}), "scalars": scalarSets, "evaluations": evaluations}
			b, err := json.Marshal(value)
			must(err)
			must(os.WriteFile(filepath.Join(*out, name), b, 0644))
			cases = append(cases, name)
		}
	}
	b, err := json.Marshal(cases)
	must(err)
	must(os.WriteFile(filepath.Join(*out, "index.json"), b, 0644))
}
func must(err error) {
	if err != nil {
		panic(err)
	}
}
