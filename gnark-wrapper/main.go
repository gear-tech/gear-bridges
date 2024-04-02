package main

import (
	"bufio"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"math/big"
	"os"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark-crypto/kzg"
	"github.com/consensys/gnark/backend/plonk"
	plonk_bn254 "github.com/consensys/gnark/backend/plonk/bn254"
	"github.com/consensys/gnark/constraint"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/scs"
	gl "github.com/succinctlabs/gnark-plonky2-verifier/goldilocks"
	"github.com/succinctlabs/gnark-plonky2-verifier/trusted_setup"
	"github.com/succinctlabs/gnark-plonky2-verifier/types"
	"github.com/succinctlabs/gnark-plonky2-verifier/variables"
	"github.com/succinctlabs/gnark-plonky2-verifier/verifier"
)

import (
	"C"
)

// How much inner circuit public inputs will be packed into single outer public input.
const PublicInputCompressionFactor = 6
const MaxInnerPublicInputBits = 32

type Plonky2VerifierCircuit struct {
	CompressedPublicInputs []frontend.Variable `gnark:",public"`

	Proof        variables.Proof
	VerifierData variables.VerifierOnlyCircuitData
	PublicInputs []gl.Variable

	CommonCircuitData types.CommonCircuitData `gnark:"-"`
}

func (c *Plonky2VerifierCircuit) Define(api frontend.API) error {
	verifierChip := verifier.NewVerifierChip(api, c.CommonCircuitData)
	verifierChip.Verify(c.Proof, c.PublicInputs, c.VerifierData)

	for i := 0; i < len(c.CompressedPublicInputs); i++ {
		compressed := frontend.Variable(0)
		for j := 0; j < PublicInputCompressionFactor; j++ {
			publicInputIdx := i*PublicInputCompressionFactor + j
			if publicInputIdx == len(c.PublicInputs) {
				break
			}

			exp := frontend.Variable(new(big.Int).Lsh(big.NewInt(1), uint((PublicInputCompressionFactor-j-1)*MaxInnerPublicInputBits)))
			compressed = api.Add(compressed, api.Mul(c.PublicInputs[publicInputIdx].Limb, exp))
		}

		api.AssertIsEqual(c.CompressedPublicInputs[i], compressed)
	}

	// TODO: Assert verifier data (also constants merkle caps?)

	return nil
}

//export compile
func compile(circuitData *C.char) {
	circuit, err := loadCircuit(C.GoString(circuitData))
	if err != nil {
		panic(err)
	}

	r1cs, err := frontend.Compile(ecc.BN254.ScalarField(), scs.NewBuilder, &circuit)
	if err != nil {
		fmt.Println("error in building circuit", err)
		os.Exit(1)
	}

	srs := loadSRS()

	pk, vk, err := plonk.Setup(r1cs, srs)
	if err != nil {
		fmt.Println(err)
		os.Exit(1)
	}

	fR1CS, _ := os.Create("data/r1cs")
	r1cs.WriteTo(fR1CS)
	fR1CS.Close()

	fPK, _ := os.Create("data/proving.key")
	pk.WriteRawTo(fPK)
	fPK.Close()

	fVK, _ := os.Create("data/verifying.key")
	vk.WriteRawTo(fVK)
	fVK.Close()

	fSolidity, _ := os.Create("data/verifier.sol")
	_ = vk.ExportSolidity(fSolidity)
}

type ProofWithPublicInputs struct {
	Proof        string     `json:"proof"`
	PublicInputs []*big.Int `json:"public_inputs"`
}

//export prove
func prove(circuitData *C.char) *C.char {
	r1cs := loadR1CS()
	pk := loadProvingKey()

	assignment, err := loadCircuit(C.GoString(circuitData))
	if err != nil {
		panic(err)
	}
	witness, _ := frontend.NewWitness(&assignment, ecc.BN254.ScalarField())

	proof, err := plonk.Prove(r1cs, pk, witness)
	if err != nil {
		panic(err)
	}

	vk := loadVerifyingKey()
	publicWitness, err := witness.Public()
	if err != nil {
		panic(err)
	}
	err = plonk.Verify(proof, vk, publicWitness)
	if err != nil {
		panic(err)
	}

	rawProof := serializeProof(proof, assignment.PublicInputs)

	return C.CString(rawProof)
}

func serializeProof(proof plonk.Proof, glPublicInputs []gl.Variable) string {
	_proof := proof.(*plonk_bn254.Proof)
	proofBytes := _proof.MarshalSolidity()
	proofStr := hex.EncodeToString(proofBytes)

	compressedPublicInputs := compressPublicInputs(glPublicInputs)
	publicInputs := make([]*big.Int, len(compressedPublicInputs))
	for i := 0; i < len(publicInputs); i++ {
		publicInputs[i] = compressedPublicInputs[i].(*big.Int)
	}

	jsonProof, err := json.MarshalIndent(ProofWithPublicInputs{
		Proof:        "0x" + proofStr,
		PublicInputs: publicInputs,
	}, "", "  ")
	if err != nil {
		panic(err)
	}

	return string(jsonProof)
}

type rawCircuit struct {
	CircuitData  string `json:"common_circuit_data"`
	Proof        string `json:"proof_with_public_inputs"`
	VerifierData string `json:"verifier_only_circuit_data"`
}

// load circuit from json
func loadCircuit(data string) (Plonky2VerifierCircuit, error) {
	handleErr := func(err error) (Plonky2VerifierCircuit, error) {
		return Plonky2VerifierCircuit{}, fmt.Errorf("error loading circuit: %w", err)
	}
	var circuit rawCircuit

	err := json.Unmarshal([]byte(data), &circuit)
	if err != nil {
		return handleErr(fmt.Errorf("unmarshal circuit data: %w", err))
	}

	var commonCircuitData types.CommonCircuitData

	{ // hack until https://github.com/succinctlabs/gnark-plonky2-verifier/pull/52 is merged
		f, err := os.CreateTemp("/tmp/", "circuit_data_*.json")
		if err != nil {
			return handleErr(fmt.Errorf("create temp file: %w", err))
		}

		_, err = io.WriteString(f, circuit.CircuitData)
		if err != nil {
			return handleErr(fmt.Errorf("write temp file: %w", err))
		}

		commonCircuitData = types.ReadCommonCircuitData(f.Name())
	}

	var rawProof types.ProofWithPublicInputsRaw
	if err := json.Unmarshal([]byte(circuit.Proof), &data); err != nil {
		return handleErr(fmt.Errorf("unmarshal proof: %w", err))
	}
	proofWithPis := variables.DeserializeProofWithPublicInputs(rawProof)

	var rawVerifierData types.VerifierOnlyCircuitDataRaw
	if err := json.Unmarshal([]byte(circuit.VerifierData), &data); err != nil {
		return handleErr(fmt.Errorf("unmarshal verifier data: %w", err))
	}
	verifierOnlyCircuitData := variables.DeserializeVerifierOnlyCircuitData(rawVerifierData)

	publicInputs := make([]gl.Variable, len(proofWithPis.PublicInputs))
	for i := 0; i < len(publicInputs); i++ {
		reduced := proofWithPis.PublicInputs[i].Limb.(uint64) % gl.MODULUS.Uint64()
		if reduced >= 1<<MaxInnerPublicInputBits {
			panic(fmt.Sprintf("Public input value too big: Expected < %d, got %d", 1<<MaxInnerPublicInputBits, reduced))
		}
		publicInputs[i] = gl.NewVariable(reduced)
	}

	return Plonky2VerifierCircuit{
		CompressedPublicInputs: compressPublicInputs(proofWithPis.PublicInputs),

		Proof:        proofWithPis.Proof,
		VerifierData: verifierOnlyCircuitData,
		PublicInputs: publicInputs,

		CommonCircuitData: commonCircuitData,
	}, nil
}

func compressPublicInputs(pis []gl.Variable) []frontend.Variable {
	compressedLen := (len(pis) + PublicInputCompressionFactor - 1) / PublicInputCompressionFactor

	compressedPis := make([]frontend.Variable, compressedLen)
	for i := 0; i < compressedLen; i++ {
		compressed := new(big.Int)
		for j := 0; j < PublicInputCompressionFactor; j++ {
			publicInputIdx := i*PublicInputCompressionFactor + j
			if publicInputIdx >= len(pis) {
				break
			}

			exp := new(big.Int).Lsh(big.NewInt(1), uint((PublicInputCompressionFactor-j-1)*MaxInnerPublicInputBits))
			publicInput := new(big.Int).SetUint64(pis[publicInputIdx].Limb.(uint64))
			compressed = new(big.Int).Add(compressed, new(big.Int).Mul(exp, publicInput))
		}
		compressedPis[i] = frontend.Variable(compressed)
	}

	return compressedPis
}

func loadVerifyingKey() plonk.VerifyingKey {
	vkFile, err := os.Open("data/verifying.key")
	if err != nil {
		fmt.Println(err)
	}
	vk := plonk.NewVerifyingKey(ecc.BN254)
	_, err = vk.ReadFrom(vkFile)
	if err != nil {
		fmt.Println(err)
	}
	vkFile.Close()

	return vk
}

func loadProvingKey() plonk.ProvingKey {
	pkFile, err := os.Open("data/proving.key")
	if err != nil {
		fmt.Println(err)
	}
	pk := plonk.NewProvingKey(ecc.BN254)
	pkReader := bufio.NewReader(pkFile)
	_, err = pk.ReadFrom(pkReader)
	if err != nil {
		fmt.Println(err)
	}
	pkFile.Close()

	return pk
}

func loadR1CS() constraint.ConstraintSystem {
	r1cs := plonk.NewCS(ecc.BN254)
	r1csFile, err := os.Open("data/r1cs")
	if err != nil {
		fmt.Println(err)
	}
	r1csReader := bufio.NewReader(r1csFile)
	_, err = r1cs.ReadFrom(r1csReader)
	if err != nil {
		fmt.Println(err)
	}
	r1csFile.Close()

	return r1cs
}

func loadSRS() kzg.SRS {
	fmt.Println("Running circuit setup")
	fileName := "data/srs_setup"
	if _, err := os.Stat(fileName); os.IsNotExist(err) {
		trusted_setup.DownloadAndSaveAztecIgnitionSrs(174, fileName)
	}
	fSRS, err := os.Open(fileName)
	if err != nil {
		panic(err)
	}
	var srs kzg.SRS = kzg.NewSRS(ecc.BN254)
	_, err = srs.ReadFrom(fSRS)
	fSRS.Close()
	if err != nil {
		panic(err)
	}

	return srs
}

func main() {}
