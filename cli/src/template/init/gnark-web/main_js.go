//go:build js && wasm

package main

import (
	"fmt"
	"strconv"
	"syscall/js"

	"github.com/consensys/gnark/logger"
	"github.com/consensys/gnark/std"
)

func byteArray(value js.Value) []byte {
	data := make([]byte, value.Get("byteLength").Int())
	js.CopyBytesToGo(data, value)
	return data
}

func optionalBytes(value js.Value) []byte {
	if value.IsNull() || value.IsUndefined() {
		return nil
	}
	return byteArray(value)
}

func proofValue(result proofResult) map[string]any {
	return map[string]any{"proof": result.Proof, "public_inputs": result.PublicInputs,
		"execution": map[string]any{"arithmetic": result.Execution.Arithmetic, "solver": result.Execution.Solver}}
}

// Keep malformed inputs from terminating the worker's Go runtime.
func callback(fn func([]js.Value) (any, error)) js.Func {
	return js.FuncOf(func(_ js.Value, args []js.Value) (response any) {
		defer func() {
			if err := recover(); err != nil {
				response = map[string]any{"error": fmt.Sprintf("gnark: %v", err)}
			}
		}()
		value, err := fn(args)
		if err != nil {
			return map[string]any{"error": err.Error()}
		}
		return map[string]any{"value": value}
	})
}

func main() {
	logger.Disable()
	// Serialized circuits do not import the packages that registered their hints
	// when they were compiled. Register gnark's built-ins in the shipping runtime.
	std.RegisterHints()
	// Handles belong to this worker and are released explicitly by the caller.
	// Register only after all requested keys have been decoded successfully.
	circuits := make(map[string]*preparedCircuit)
	var nextID uint64
	lookup := func(id string) (*preparedCircuit, error) {
		circuit, ok := circuits[id]
		if !ok {
			return nil, fmt.Errorf("prepared gnark circuit has been disposed or does not exist")
		}
		return circuit, nil
	}
	js.Global().Set("__moproGnark", map[string]any{
		"prepare": callback(func(args []js.Value) (any, error) {
			if len(args) != 3 {
				return nil, fmt.Errorf("prepare expects R1CS and optional proving/verifying keys")
			}
			circuit, err := prepareCircuit(byteArray(args[0]), optionalBytes(args[1]), optionalBytes(args[2]))
			if err != nil {
				return nil, err
			}
			nextID++
			id := strconv.FormatUint(nextID, 10)
			circuits[id] = circuit
			return id, nil
		}),
		"provePrepared": callback(func(args []js.Value) (any, error) {
			if len(args) != 2 {
				return nil, fmt.Errorf("provePrepared expects a circuit handle and witness JSON")
			}
			circuit, err := lookup(args[0].String())
			if err != nil {
				return nil, err
			}
			result, err := circuit.prove(args[1].String())
			return proofValue(result), err
		}),
		"verifyPrepared": callback(func(args []js.Value) (any, error) {
			if len(args) != 3 {
				return nil, fmt.Errorf("verifyPrepared expects a circuit handle, proof and public inputs")
			}
			circuit, err := lookup(args[0].String())
			if err != nil {
				return nil, err
			}
			return circuit.verify(proofResult{Proof: args[1].String(), PublicInputs: args[2].String()})
		}),
		"release": callback(func(args []js.Value) (any, error) {
			if len(args) != 1 {
				return nil, fmt.Errorf("release expects a circuit handle")
			}
			if circuit, ok := circuits[args[0].String()]; ok {
				circuit.close()
			}
			delete(circuits, args[0].String())
			return nil, nil
		}),
		"prove": callback(func(args []js.Value) (any, error) {
			if len(args) != 3 {
				return nil, fmt.Errorf("prove expects R1CS, proving key and witness JSON")
			}
			result, err := prove(byteArray(args[0]), byteArray(args[1]), args[2].String())
			return proofValue(result), err
		}),
		"verify": callback(func(args []js.Value) (any, error) {
			if len(args) != 4 {
				return nil, fmt.Errorf("verify expects R1CS, verifying key, proof and public inputs")
			}
			return verify(byteArray(args[0]), byteArray(args[1]), proofResult{
				Proof: args[2].String(), PublicInputs: args[3].String(),
			})
		}),
	})
	select {}
}
