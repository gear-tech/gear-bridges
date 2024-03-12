package main

import (
	"bufio"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
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

type Plonky2VerifierCircuit struct {
	PublicInputs []gl.Variable `gnark:",public"`

	Proof        variables.Proof
	VerifierData variables.VerifierOnlyCircuitData

	CommonCircuitData types.CommonCircuitData `gnark:"-"`
}

func (c *Plonky2VerifierCircuit) Define(api frontend.API) error {
	verifierChip := verifier.NewVerifierChip(api, c.CommonCircuitData)
	verifierChip.Verify(c.Proof, c.PublicInputs, c.VerifierData)

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
	Proof        string   `json:"proof"`
	PublicInputs []uint64 `json:"public_inputs"`
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

	publicInputs := make([]uint64, len(glPublicInputs))
	for i := 0; i < len(publicInputs); i++ {
		publicInputs[i] = glPublicInputs[i].Limb.(uint64)
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

	return Plonky2VerifierCircuit{
		Proof:             proofWithPis.Proof,
		PublicInputs:      proofWithPis.PublicInputs,
		VerifierData:      verifierOnlyCircuitData,
		CommonCircuitData: commonCircuitData,
	}
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
