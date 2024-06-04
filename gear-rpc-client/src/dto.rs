const ED25519_PUBLIC_KEY_SIZE: usize = 32;
const ED25519_SIGNATURE_SIZE: usize = 64;

pub struct PreCommit {
    pub public_key: [u8; ED25519_PUBLIC_KEY_SIZE],
    pub signature: [u8; ED25519_SIGNATURE_SIZE],
}

pub struct BlockFinalityProof {
    pub validator_set: Vec<[u8; ED25519_PUBLIC_KEY_SIZE]>,
    pub pre_commits: Vec<PreCommit>,
    pub message: Vec<u8>,
}

pub struct BranchNodeData {
    pub data: Vec<u8>,
    pub target_child: u8,
}

pub struct StorageInclusionProof {
    pub address: Vec<u8>,

    pub block_header: Vec<u8>,
    /// Arranged from leaf to root.
    pub branch_nodes_data: Vec<BranchNodeData>,
    pub leaf_node_data: Vec<u8>,

    pub stored_data: Vec<u8>,
}

pub struct ValidatorSetChangeProof {
    pub current_epoch_block_finality: BlockFinalityProof,
    pub queued_keys_inclusion_proof: StorageInclusionProof,
}

pub struct MessageSentProof {
    pub block_finality_proof: BlockFinalityProof,
    pub storage_inclusion_proof: StorageInclusionProof,
}
