use prover::proving::{CircuitData, Proof, ProofWithCircuitData};

mod file_system;
mod gear;
mod in_memory;

pub use file_system::FileSystemProofStorage;
pub use gear::GearProofStorage;
use in_memory::InMemoryProofStorage;

#[derive(Debug, thiserror::Error)]
pub enum ProofStorageError {
    #[error("Proof storage already initialized")]
    AlreadyInitialized,
    #[error("Proof storage not initialized")]
    NotInitialized,
    #[error("Proof for authority set id #{0} not found")]
    NotFound(u64),
    #[error("Authority set id is not as expected")]
    AuthoritySetIdMismatch,
    #[error(transparent)]
    InnerError(#[from] anyhow::Error),
}

type AuthoritySetId = u64;


#[async_trait::async_trait]
pub trait ProofStorage: Send + Sync {
    async fn init(
        &self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError>;

    async fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError>;

    async fn get_latest_authority_set_id(&self) -> Option<AuthoritySetId>;

    async fn get_proof_for_authority_set_id(
        &self,
        authority_set_id: AuthoritySetId,
    ) -> Result<ProofWithCircuitData, ProofStorageError>;

    async fn update(
        &self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError>;
}
