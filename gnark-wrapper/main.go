package main

import (
	"bufio"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
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

func main() {
	compileCircuit := flag.Bool("compile-circuit", false, "create proving key, verifying key, R1CS and solidity verifier")
	flag.Parse()

	if *compileCircuit {
		compile()
	}

	prove()
}

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

func compile() {
	circuit := loadCircuit()

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

func prove() {
	r1cs := loadR1CS()
	pk := loadProvingKey()

	assignment := loadCircuit()
	witness, _ := frontend.NewWitness(&assignment, ecc.BN254.ScalarField())

	proof, err := plonk.Prove(r1cs, pk, witness)
	if err != nil {
		panic(err)
	}

	saveProof(proof, assignment.PublicInputs)

	vk := loadVerifyingKey()
	publicWitness, err := witness.Public()
	if err != nil {
		panic(err)
	}
	err = plonk.Verify(proof, vk, publicWitness)
	if err != nil {
		panic(err)
	}
}

func saveProof(proof plonk.Proof, glPublicInputs []gl.Variable) {
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

	err = os.WriteFile("data/final_proof.json", jsonProof, 0644)
	if err != nil {
		panic(err)
	}
}

func loadCircuit() Plonky2VerifierCircuit {
	commonCircuitData := types.ReadCommonCircuitData("data/common_circuit_data.json")
	proofWithPis := variables.DeserializeProofWithPublicInputs(types.ReadProofWithPublicInputs("data/proof_with_public_inputs.json"))
	verifierOnlyCircuitData := variables.DeserializeVerifierOnlyCircuitData(types.ReadVerifierOnlyCircuitData("data/verifier_only_circuit_data.json"))

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
	}
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
