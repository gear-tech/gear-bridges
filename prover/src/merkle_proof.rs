use sp_core::blake2_256;

pub struct TrieNodeData {
    pub left_data: Vec<u8>,
    pub right_data: Vec<u8>,
}

pub struct MerkleProof {
    /// Ordered from leaf to the root.
    pub nodes: Vec<TrieNodeData>,
    pub leaf_data: Vec<u8>,
    pub root_hash: [u8; 32],
}

impl MerkleProof {
    pub fn prove(&self) {
        // TODO: zk

        let mut hash = blake2_256(&self.leaf_data);

        for node in &self.nodes {
            let mut hash_in = node.left_data.clone();
            hash_in.append(&mut hash.to_vec());
            hash_in.append(&mut node.right_data.clone());

            hash = blake2_256(&hash_in);
        }

        println!(
            "computed root: {:?}, state root: {:?}",
            hash, self.root_hash
        );
    }
}
