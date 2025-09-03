use primitive_types::H256;
use tokio::sync::oneshot::Sender;

mod merkle_roots {
    tonic::include_proto!("merkle_roots");
}

pub enum MerkleRootsRequest {
    GetMerkleRootProof {
        block_number: u32,
        response: Sender<MerkleRootsResponse>,
    },
}

pub enum MerkleRootsResponse {
    MerkleRootProof {
        proof: Vec<u8>,
        merkle_root: H256,
        block_number: u32,
        block_hash: H256,
    },

    NoMerkleRootOnBlock {
        block_number: u32,
    },

    Failed {
        message: String,
    },
}

#[cfg(feature = "server")]
pub mod server {
    use super::*;

    use tokio::sync::{
        mpsc,
        oneshot::{self},
    };
    use tonic::{
        metadata::{Ascii, MetadataValue},
        Request, Response, Status,
    };

    use crate::merkle_roots::merkle_roots::{
        merkle_root_proof_response, MerkleRootProof, MerkleRootProofRequest,
        MerkleRootProofResponse,
    };

    pub use merkle_roots::merkle_roots_server::MerkleRootsServer;

    pub struct MerkleRoots {
        requests: mpsc::UnboundedSender<MerkleRootsRequest>,
        auth_token: MetadataValue<Ascii>,
    }

    impl MerkleRoots {
        pub fn new(
            requests: mpsc::UnboundedSender<MerkleRootsRequest>,
            auth_token: String,
        ) -> Self {
            let auth_token = auth_token.parse().expect("Auth token is not valid ASCII");
            Self {
                requests,
                auth_token,
            }
        }
    }

    #[tonic::async_trait]
    impl merkle_roots::merkle_roots_server::MerkleRoots for MerkleRoots {
        async fn get_merkle_root_proof(
            &self,
            request: Request<MerkleRootProofRequest>,
        ) -> Result<Response<MerkleRootProofResponse>, Status> {
            match request.metadata().get("authorization") {
                Some(t) if *t == self.auth_token => (),
                _ => return Err(Status::unauthenticated("Invalid auth token")),
            }

            let (tx, rx) = oneshot::channel();
            match self.requests.send(MerkleRootsRequest::GetMerkleRootProof {
                block_number: request.get_ref().block_number,
                response: tx,
            }) {
                Ok(_) => (),
                Err(_) => return Err(Status::unavailable("Service is unavailable")),
            }

            match rx.await {
                Ok(response) => match response {
                    MerkleRootsResponse::MerkleRootProof {
                        proof,
                        block_number,
                        block_hash,
                        merkle_root,
                    } => Ok(MerkleRootProofResponse {
                        response: Some(merkle_root_proof_response::Response::Proof(
                            MerkleRootProof {
                                block_hash: block_hash.as_bytes().to_vec(),
                                proof,
                                block_number,
                                merkle_root: merkle_root.as_bytes().to_vec(),
                            },
                        )),
                    }
                    .into()),

                    MerkleRootsResponse::NoMerkleRootOnBlock { block_number } => {
                        Ok(MerkleRootProofResponse {
                            response: Some(
                                merkle_root_proof_response::Response::NoMerkleRootOnBlock(
                                    block_number,
                                ),
                            ),
                        }
                        .into())
                    }

                    MerkleRootsResponse::Failed { message } => Ok(MerkleRootProofResponse {
                        response: Some(merkle_root_proof_response::Response::Failed(message)),
                    }
                    .into()),
                },

                Err(_) => Err(Status::unavailable("Service is unavailable")),
            }
        }
    }
}

#[cfg(feature = "client")]
pub mod client {
    pub use super::merkle_roots::{
        merkle_roots_client::MerkleRootsClient, MerkleRootProofRequest, MerkleRootProofResponse,
    };
}
