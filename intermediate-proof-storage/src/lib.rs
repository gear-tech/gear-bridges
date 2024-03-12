pub enum ProofStorageError {
    CannotAcceptCircuitData,
    AlreadyInitialized,
    WrongProof,
    WrongValidatorSetId,
}

pub trait ProofStorage {
    fn init(
        &self,
        common_circuit_data: String,
        verifier_only_circuit_data: String,
        proof_with_public_inputs: String,
    ) -> Result<(), ProofStorageError>;

    fn get_latest_proof(&self) -> Result<Option<String>, ProofStorageError>;

    fn update(&self, proof_with_public_inputs: String) -> Result<(), ProofStorageError>;
}

struct MockProofStorage {
    latest_proof: String,
}

impl ProofStorage for MockProofStorage {
    fn init(
        &self,
        common_circuit_data: String,
        verifier_only_circuit_data: String,
        proof_with_public_inputs: String,
    ) -> Result<(), ProofStorageError> {
        todo!()
    }

    fn get_latest_proof(&self) -> Result<Option<String>, ProofStorageError> {
        todo!()
    }

    fn update(&self, proof_with_public_inputs: String) -> Result<(), ProofStorageError> {
        todo!()
    }
}

struct VaraProofStorage {}

impl ProofStorage for VaraProofStorage {
    fn init(
        &self,
        common_circuit_data: String,
        verifier_only_circuit_data: String,
        proof_with_public_inputs: String,
    ) -> Result<(), ProofStorageError> {
        todo!()
    }

    fn get_latest_proof(&self) -> Result<Option<String>, ProofStorageError> {
        todo!()
    }

    fn update(&self, proof_with_public_inputs: String) -> Result<(), ProofStorageError> {
        todo!()
    }
}
