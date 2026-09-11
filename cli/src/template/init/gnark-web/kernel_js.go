//go:build js && wasm

package main

import (
	"bytes"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"syscall/js"

	curve "github.com/consensys/gnark-crypto/ecc/bn254"
	"github.com/consensys/gnark-crypto/ecc/bn254/fr"
	native "github.com/consensys/gnark/backend/groth16/bn254"
)

type browserKernel struct {
	key js.Value
}

func prepareKernel(c *preparedCircuit) {
	// Small circuits cost less in the existing Go implementation.
	module := js.Global().Get("__moproGnarkKernel")
	if c.pk == nil || c.cs.GetNbConstraints() < 1024 || module.Type() != js.TypeObject {
		return
	}
	constructor := module.Get("Key")
	if constructor.Type() != js.TypeFunction {
		return
	}
	pk := c.pk.(*native.ProvingKey)
	kernel := &browserKernel{key: constructor.New(packKernel(pk.G1.A), packKernel(pk.G1.B), packKernel(pk.G1.K), packKernel(pk.G1.Z), packKernel(pk.G2.B), packKernel([]fr.Element{pk.Domain.Generator, pk.Domain.FrMultiplicativeGen}))}
	installed := false
	defer func() {
		if !installed {
			kernel.close()
		}
	}()
	for _, commitment := range pk.CommitmentKeys {
		kernel.key.Call("add_commitment", packKernel(commitment.Basis), packKernel(commitment.BasisExpSigma))
	}
	if kernel.key.Get("initialize").Type() == js.TypeFunction {
		if _, err := awaitKernel(kernel.key.Call("initialize")); err != nil {
			panic(err)
		}
	}
	c.kernel = kernel
	installed = true
}

// Gnark and arkworks use the same four little-endian Montgomery limbs.
// Only decoded keys and field elements cross this bridge.
func packKernel(value any) js.Value {
	var buf bytes.Buffer
	if err := binary.Write(&buf, binary.LittleEndian, value); err != nil {
		panic(err)
	}
	out := js.Global().Get("Uint8Array").New(buf.Len())
	js.CopyBytesToJS(out, buf.Bytes())
	return out
}

func packKernelFields(values []fr.Element) js.Value {
	data := encodeKernelFields(values)
	out := js.Global().Get("Uint8Array").New(len(data))
	js.CopyBytesToJS(out, data)
	return out
}

func (k *browserKernel) parts(sa, sb, sk, a, b, c []fr.Element) (kernelParts, error) {
	result := k.key.Call("parts", packKernelFields(sa), packKernelFields(sb), packKernelFields(sk), packKernelFields(a), packKernelFields(b), packKernelFields(c))
	result, err := awaitKernel(result)
	if err != nil {
		return kernelParts{}, err
	}
	return unpackKernelParts(result)
}
func unpackKernelParts(result js.Value) (kernelParts, error) {
	var out kernelParts
	data := byteArray(result)
	if len(data) != 384 {
		return out, fmt.Errorf("invalid accelerated proof parts")
	}
	reader := bytes.NewReader(data)
	for _, point := range []any{&out.a, &out.b, &out.k, &out.z, &out.b2} {
		if err := binary.Read(reader, binary.LittleEndian, point); err != nil {
			return out, err
		}
	}
	return out, nil
}
func (k *browserKernel) close() { k.key.Call("free") }

func (k *browserKernel) commitment(index int, knowledge bool, values []fr.Element) (curve.G1Affine, error) {
	var out curve.G1Affine
	result, err := awaitKernel(k.key.Call("commitment", index, knowledge, packKernelFields(values)))
	if err != nil {
		return out, err
	}
	data := byteArray(result)
	if len(data) != 64 {
		return out, fmt.Errorf("invalid accelerated commitment")
	}
	err = binary.Read(bytes.NewReader(data), binary.LittleEndian, &out)
	return out, err
}

// Called from the goroutine behind callback, never a blocking JS callback.
func awaitKernel(value js.Value) (js.Value, error) {
	if value.Type() != js.TypeObject || value.Get("then").Type() != js.TypeFunction {
		return value, nil
	}
	type response struct {
		value js.Value
		err   error
	}
	done := make(chan response, 1)
	ok := js.FuncOf(func(_ js.Value, args []js.Value) any { done <- response{value: args[0]}; return nil })
	fail := js.FuncOf(func(_ js.Value, args []js.Value) any {
		done <- response{err: fmt.Errorf("arithmetic backend: %s", args[0].String())}
		return nil
	})
	defer ok.Release()
	defer fail.Release()
	value.Call("then", ok, fail)
	result := <-done
	return result.value, result.err
}
func kernelProfile(kernel proofKernel) map[string]float64 {
	if k, ok := kernel.(*browserKernel); ok && k.key.Get("profile").Type() == js.TypeFunction {
		var result map[string]float64
		if json.Unmarshal([]byte(k.key.Call("profile").String()), &result) == nil {
			return result
		}
	}
	return nil
}
