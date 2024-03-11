package main

import (
	"bufio"
	"encoding/hex"
	"flag"
	"fmt"
	"os"
	"time"

	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark-crypto/kzg"
	"github.com/consensys/gnark/backend/plonk"
	plonk_bn254 "github.com/consensys/gnark/backend/plonk/bn254"
	"github.com/consensys/gnark/constraint"
	"github.com/consensys/gnark/frontend"
	"github.com/consensys/gnark/frontend/cs/scs"
	"github.com/consensys/gnark/profile"
	"github.com/consensys/gnark/test"
	gl "github.com/succinctlabs/gnark-plonky2-verifier/goldilocks"
	"github.com/succinctlabs/gnark-plonky2-verifier/trusted_setup"
	"github.com/succinctlabs/gnark-plonky2-verifier/types"
	"github.com/succinctlabs/gnark-plonky2-verifier/variables"
	"github.com/succinctlabs/gnark-plonky2-verifier/verifier"
)

type ExampleVerifierCircuit struct {
	PublicInputs []gl.Variable `gnark:",public"`

	// Private inputs to the circuit
	Proof        variables.Proof
	VerifierData variables.VerifierOnlyCircuitData

	// Circuit configuration that is not part of the circuit itself.
	CommonCircuitData types.CommonCircuitData `gnark:"-"`
}

func (c *ExampleVerifierCircuit) Define(api frontend.API) error {
	verifierChip := verifier.NewVerifierChip(api, c.CommonCircuitData)
	verifierChip.Verify(c.Proof, c.PublicInputs, c.VerifierData)

	return nil
}

func runBenchmark(plonky2Circuit string, proofSystem string, profileCircuit bool, dummy bool, saveArtifacts bool) {
	if proofSystem == "plonk" {
		commonCircuitData := types.ReadCommonCircuitData("testdata/" + plonky2Circuit + "/common_circuit_data.json")
		proofWithPis := variables.DeserializeProofWithPublicInputs(types.ReadProofWithPublicInputs("testdata/" + plonky2Circuit + "/proof_with_public_inputs.json"))
		verifierOnlyCircuitData := variables.DeserializeVerifierOnlyCircuitData(types.ReadVerifierOnlyCircuitData("testdata/" + plonky2Circuit + "/verifier_only_circuit_data.json"))

		circuit := ExampleVerifierCircuit{
			Proof:             proofWithPis.Proof,
			PublicInputs:      proofWithPis.PublicInputs,
			VerifierData:      verifierOnlyCircuitData,
			CommonCircuitData: commonCircuitData,
		}

		var p *profile.Profile
		if profileCircuit {
			p = profile.Start()
		}

		r1cs, err := frontend.Compile(ecc.BN254.ScalarField(), scs.NewBuilder, &circuit)
		if err != nil {
			fmt.Println("error in building circuit", err)
			os.Exit(1)
		}

		if profileCircuit {
			p.Stop()
			p.Top()
			println("r1cs.GetNbCoefficients(): ", r1cs.GetNbCoefficients())
			println("r1cs.GetNbConstraints(): ", r1cs.GetNbConstraints())
			println("r1cs.GetNbSecretVariables(): ", r1cs.GetNbSecretVariables())
			println("r1cs.GetNbPublicVariables(): ", r1cs.GetNbPublicVariables())
			println("r1cs.GetNbInternalVariables(): ", r1cs.GetNbInternalVariables())
		}

		plonkProof(r1cs, plonky2Circuit, dummy, saveArtifacts)
	} else if proofSystem == "plonk_prove_only" {
		plonkProveOnly(plonky2Circuit, saveArtifacts)
	} else {
		panic("Please provide a valid proof system to benchmark, we only support plonk and groth16")
	}
}

func plonkProof(r1cs constraint.ConstraintSystem, circuitName string, dummy bool, saveArtifacts bool) {
	var pk plonk.ProvingKey
	var vk plonk.VerifyingKey
	var srs kzg.SRS = kzg.NewSRS(ecc.BN254)
	var err error

	proofWithPis := variables.DeserializeProofWithPublicInputs(types.ReadProofWithPublicInputs("testdata/" + circuitName + "/proof_with_public_inputs.json"))
	verifierOnlyCircuitData := variables.DeserializeVerifierOnlyCircuitData(types.ReadVerifierOnlyCircuitData("testdata/" + circuitName + "/verifier_only_circuit_data.json"))
	assignment := ExampleVerifierCircuit{
		Proof:        proofWithPis.Proof,
		PublicInputs: proofWithPis.PublicInputs,
		VerifierData: verifierOnlyCircuitData,
	}

	if saveArtifacts {
		fR1CS, _ := os.Create("circuit")
		r1cs.WriteTo(fR1CS)
		fR1CS.Close()
	}

	fmt.Println("Running circuit setup", time.Now())
	if dummy {
		fmt.Println("Using test setup")

		srs, err = test.NewKZGSRS(r1cs)

		if err != nil {
			panic(err)
		}
	} else {
		fmt.Println("Using real setup")

		fileName := "srs_setup"

		if _, err := os.Stat(fileName); os.IsNotExist(err) {
			trusted_setup.DownloadAndSaveAztecIgnitionSrs(174, fileName)
		}

		fSRS, err := os.Open(fileName)
		if err != nil {
			panic(err)
		}

		_, err = srs.ReadFrom(fSRS)

		fSRS.Close()

		if err != nil {
			panic(err)
		}
	}

	pk, vk, err = plonk.Setup(r1cs, srs)

	if err != nil {
		fmt.Println(err)
		os.Exit(1)
	}

	if saveArtifacts {
		fPK, _ := os.Create("proving.key")
		pk.WriteRawTo(fPK)
		fPK.Close()

		if vk != nil {
			fVK, _ := os.Create("verifying.key")
			vk.WriteRawTo(fVK)
			fVK.Close()
		}

		fSolidity, _ := os.Create("proof.sol")
		err = vk.ExportSolidity(fSolidity)
	}

	fmt.Println("Generating witness", time.Now())
	witness, _ := frontend.NewWitness(&assignment, ecc.BN254.ScalarField())
	publicWitness, _ := witness.Public()
	// if saveArtifacts {
	// 	fWitness, _ := os.Create("witness")
	// 	witness.WriteTo(fWitness)
	// 	fWitness.Close()
	// }

	fmt.Println("Creating proof", time.Now())
	proof, err := plonk.Prove(r1cs, pk, witness)
	if err != nil {
		fmt.Println(err)
		os.Exit(1)
	}
	// if saveArtifacts {
	// 	fProof, _ := os.Create("proof.proof")
	// 	proof.WriteTo(fProof)
	// 	fProof.Close()
	// }

	if vk == nil {
		fmt.Println("vk is nil, means you're using dummy setup and we skip verification of proof")
		return
	}

	fmt.Println("Verifying proof", time.Now())
	err = plonk.Verify(proof, vk, publicWitness)
	if err != nil {
		fmt.Println(err)
		os.Exit(1)
	}

	_proof := proof.(*plonk_bn254.Proof)
	proofBytes := _proof.MarshalSolidity()
	proofStr := hex.EncodeToString(proofBytes)
	fmt.Println(proofStr)
}

func plonkProveOnly(circuitName string, saveArtifacts bool) {
	var err error

	proofWithPis := variables.DeserializeProofWithPublicInputs(types.ReadProofWithPublicInputs("testdata/" + circuitName + "/proof_with_public_inputs.json"))
	verifierOnlyCircuitData := variables.DeserializeVerifierOnlyCircuitData(types.ReadVerifierOnlyCircuitData("testdata/" + circuitName + "/verifier_only_circuit_data.json"))
	assignment := ExampleVerifierCircuit{
		Proof:        proofWithPis.Proof,
		PublicInputs: proofWithPis.PublicInputs,
		VerifierData: verifierOnlyCircuitData,
	}

	r1cs := plonk.NewCS(ecc.BN254)
	r1csFile, err := os.Open("circuit")
	if err != nil {
		fmt.Println(err)
	}
	r1csReader := bufio.NewReader(r1csFile)
	_, err = r1cs.ReadFrom(r1csReader)
	if err != nil {
		fmt.Println(err)
	}
	r1csFile.Close()

	pkFile, err := os.Open("proving.key")
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

	vkFile, err := os.Open("verifying.key")
	if err != nil {
		fmt.Println(err)
	}
	vk := plonk.NewVerifyingKey(ecc.BN254)
	_, err = vk.ReadFrom(vkFile)
	if err != nil {
		fmt.Println(err)
	}
	vkFile.Close()

	fmt.Println("Generating witness", time.Now())
	witness, _ := frontend.NewWitness(&assignment, ecc.BN254.ScalarField())
	publicWitness, _ := witness.Public()

	fmt.Println("Creating proof", time.Now())
	proof, err := plonk.Prove(r1cs, pk, witness)
	if err != nil {
		fmt.Println(err)
		os.Exit(1)
	}

	fmt.Println("Verifying proof", time.Now())
	err = plonk.Verify(proof, vk, publicWitness)
	if err != nil {
		fmt.Println(err)
		os.Exit(1)
	}

	_proof := proof.(*plonk_bn254.Proof)
	proofBytes := _proof.MarshalSolidity()
	proofStr := hex.EncodeToString(proofBytes)
	fmt.Println(proofStr)
}

func main() {
	plonky2Circuit := flag.String("plonky2-circuit", "own_2", "plonky2 circuit to benchmark")
	proofSystem := flag.String("proof-system", "plonk_prove_only", "proof system to benchmark")
	profileCircuit := flag.Bool("profile", true, "profile the circuit")
	dummySetup := flag.Bool("dummy", false, "use dummy setup")
	saveArtifacts := flag.Bool("save", true, "save circuit artifacts")

	flag.Parse()

	if plonky2Circuit == nil || *plonky2Circuit == "" {
		fmt.Println("Please provide a plonky2 circuit to benchmark")
		os.Exit(1)
	}

	fmt.Printf("Running benchmark for %s circuit with proof system %s\n", *plonky2Circuit, *proofSystem)
	fmt.Printf("Profiling: %t, DummySetup: %t, SaveArtifacts: %t\n", *profileCircuit, *dummySetup, *saveArtifacts)

	runBenchmark(*plonky2Circuit, *proofSystem, *profileCircuit, *dummySetup, *saveArtifacts)
}
