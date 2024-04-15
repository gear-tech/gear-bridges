extern crate core;

mod abi;
mod convert;
pub mod error;
mod msg;
mod proof;

use crate::abi::IMessageQueue::IMessageQueueInstance;
use crate::abi::IRelayer::IRelayerInstance;
use crate::abi::{IMessageQueue, IRelayer};
use crate::convert::Convert;
use crate::proof::Proof;
use alloy_contract::Event;
use alloy_network::{Ethereum, EthereumSigner, Network, TxSigner};
use alloy_primitives::{hex, Address, Bytes, TxHash, B256, U256};
use alloy_provider::layers::{
    GasEstimatorLayer, GasEstimatorProvider, ManagedNonceLayer, ManagedNonceProvider,
    SignerProvider,
};
use alloy_provider::{Provider, ProviderBuilder, ProviderLayer, RootProvider};
use alloy_rpc_client::RpcClient;
use alloy_rpc_types::{Filter, TransactionRequest};
use alloy_signer::k256::ecdsa::SigningKey;
use alloy_signer_wallet::{LocalWallet, Wallet};
use alloy_sol_types::{sol, SolCall, SolEvent, SolInterface};
use alloy_transport::{BoxTransport, Transport};
use error::VerifierError;
use reqwest::{Client, Url};
use serde::Deserialize;
use sp_runtime::traits::Keccak256;
use std::marker::PhantomData;
use std::sync::Arc;

type ProviderType = ManagedNonceProvider<
    Ethereum,
    BoxTransport,
    GasEstimatorProvider<
        Ethereum,
        BoxTransport,
        SignerProvider<
            Ethereum,
            BoxTransport,
            RootProvider<Ethereum, BoxTransport>,
            EthereumSigner,
        >,
    >,
>;

pub struct ContractVerifiers {
    signer: Wallet<SigningKey>,
    provider: Arc<ProviderType>,
    message_queue_instance: IMessageQueueInstance<Ethereum, BoxTransport, Arc<ProviderType>>,
    relayer_instance: IRelayerInstance<Ethereum, BoxTransport, Arc<ProviderType>>,
}

impl ContractVerifiers {
    pub fn new(
        url: &str,
        message_queue_address_str: &str,
        relayer_address_str: &str,
        pk_str: Option<&str>,
    ) -> Result<Self, VerifierError> {
        let http = alloy_transport_http::Http::<Client>::new(Url::parse(url).unwrap()).boxed();
        let rp = RootProvider::<Ethereum, BoxTransport>::new(RpcClient::new(http, true));

        let signer = match pk_str {
            Some(pk) => {
                let pk = hex::decode(pk).map_err(|_| VerifierError::WrongPrivateKey)?;
                LocalWallet::from_bytes(&B256::from_slice(pk.as_slice()))
                    .map_err(|_| VerifierError::WrongPrivateKey)?
            }
            None => LocalWallet::random(),
        };

        let provider = Arc::new(
            ProviderBuilder::new()
                .layer(ManagedNonceLayer)
                .layer(GasEstimatorLayer)
                .signer(EthereumSigner::from(signer.clone())) // note the order!
                .provider(rp),
        );

        let relayer_contract_address: Address = relayer_address_str
            .parse()
            .map_err(|_| VerifierError::WrongAddress)?;
        let message_queue_contract_address: Address = message_queue_address_str
            .parse()
            .map_err(|_| VerifierError::WrongAddress)?;

        let relayer_instance = IRelayer::new(relayer_contract_address, provider.clone());
        let message_queue_instance =
            IMessageQueue::new(message_queue_contract_address, provider.clone());

        Ok(ContractVerifiers {
            signer,
            provider,
            relayer_instance,
            message_queue_instance,
        })
    }

    pub async fn verify_merkle_root_tx<U: Convert<U256>, B: Convert<Bytes>>(
        &self,
        public_inputs: Vec<U>,
        proof: B,
    ) -> Result<bool, VerifierError> {
        let public_inputs: Vec<U256> = public_inputs.into_iter().map(|v| v.convert()).collect();
        let proof = proof.convert();

        match self
            .relayer_instance
            .submit_merkle_root(public_inputs.clone(), proof.clone())
            .estimate_gas()
            .await
        {
            Ok(gas_used) => {
                println!("Gas used: {gas_used}");
                match self
                    .relayer_instance
                    .submit_merkle_root(public_inputs, proof)
                    .from(self.signer.address())
                    .gas_price(U256::from(2_000_000_000_000u128))
                    .send()
                    .await
                {
                    Ok(pending_tx) => match pending_tx.get_receipt().await {
                        Ok(receipt) => Ok(true),
                        Err(e) => Err(VerifierError::ErrorWaitingTransactionReceipt),
                    },
                    Err(e) => {
                        println!("Sending error: {e:?}");
                        Err(VerifierError::ErrorSendingTransaction)
                    }
                }
            }
            Err(e) => Err(VerifierError::ErrorDuringContractExecution),
        }
    }

    pub async fn verify_merkle_root_from_json_string(
        &self,
        json_string: &str,
    ) -> Result<bool, VerifierError> {
        let proof: Proof = Proof::try_from_json_string(json_string)
            .map_err(|_| VerifierError::WrongJsonFormation)?;
        self.verify_merkle_root_tx(proof.public_inputs, proof.proof)
            .await
    }

    pub async fn fetch_merkle_roots(
        &self,
        depth: u64,
    ) -> Result<Vec<(u64, B256, TxHash)>, VerifierError> {
        let current_block: u64 = self
            .provider
            .get_block_number()
            .await
            .map_err(|_| VerifierError::ErrorInHTTPTransport)?;

        let filter = Filter::new()
            .address(*self.relayer_instance.address())
            .event_signature(IRelayer::MerkleRoot::SIGNATURE_HASH)
            .from_block(current_block.checked_sub(depth).unwrap_or_default());

        let event: Event<_, _, _, IRelayer::MerkleRoot> = Event::new(self.provider.clone(), filter);

        match event.query().await {
            Ok(logs) => Ok(logs
                .iter()
                .map(|(event, log)| {
                    (
                        event.blockNumber.to(),
                        event.merkleRoot,
                        log.transaction_hash.unwrap_or_default(),
                    )
                })
                .collect()),
            Err(_) => Err(VerifierError::ErrorInHTTPTransport),
        }
    }

    pub async fn verify_msg_sent(
        &self,
        pk: String,
        path_to_final_proof: &str,
        path_to_final_public: &str,
    ) -> Result<bool, VerifierError> {
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use crate::msg::ContentMessage;
    use crate::*;
    use alloy_node_bindings::{Anvil, AnvilInstance};
    use alloy_primitives::keccak256;
    use alloy_provider::HttpProvider;
    use alloy_transport_http::Http;
    use binary_merkle_tree::{merkle_proof, merkle_root, verify_proof, Leaf, MerkleProof};
    use primitive_types::H256;
    use sp_runtime::app_crypto::sp_core::keccak_256;

    #[allow(unused, unreachable_pub)]
    pub fn spawn_anvil() -> (HttpProvider<Ethereum>, AnvilInstance) {
        spawn_anvil_with(std::convert::identity)
    }

    #[allow(unused, unreachable_pub)]
    pub fn spawn_anvil_with(
        f: impl FnOnce(Anvil) -> Anvil,
    ) -> (HttpProvider<Ethereum>, AnvilInstance) {
        let anvil = f(Anvil::new()).try_spawn().expect("could not spawn anvil");
        (anvil_http_provider(&anvil), anvil)
    }

    #[allow(unused, unreachable_pub)]
    pub fn anvil_http_provider(anvil: &AnvilInstance) -> HttpProvider<Ethereum> {
        http_provider(&anvil.endpoint())
    }

    #[allow(unused, unreachable_pub)]
    pub fn http_provider(url: &str) -> HttpProvider<Ethereum> {
        let url = url.parse().unwrap();
        let http = Http::<Client>::new(url);
        HttpProvider::new(RpcClient::new(http, true))
    }

    #[tokio::test]
    async fn contract_verifier_create() {
        let message_queue = "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9";
        let replayer = "0x5FC8d32690cc91D4c39d9d3abcBD16989F875707";
        let pk = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let url = "http://127.0.0.1:8545/";
        let client = ContractVerifiers::new(url, message_queue, replayer, Some(pk)).unwrap();
    }

    #[tokio::test]
    async fn verify_block() {
        let message_queue = "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9";
        let replayer = "0x5FC8d32690cc91D4c39d9d3abcBD16989F875707";
        let pk = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let url = "http://127.0.0.1:8545/";
        let client = ContractVerifiers::new(url, message_queue, replayer, Some(pk)).unwrap();

        let proof_json = r#"{
                "proof" : "18d39978105e6371129a8c670c4958719bf0b860646c2dd760a14c6b5aa04b8e1682aec235c07cc291c2bc14670ab30db45b6c6ce53e7d6e42d5d4837a6a0120183d34eb74c7afdf6d88b54e1bde6948e7f566f6cc374e8bec0ab5553e2b95392ecb009497004b9defb864e8756bbfc830dc0e1f505687c9c4779a32f6783943262140c77797264ea54462073603c736a6c78b20a3016f5493f5cf95556ee81e29ed533dc33499c78e45b8c3c36993a6ad812b7073d8f4ca1a61da68b44e28d00cca5e1481a5bf5fea36beae27af01d45bf45ae9d239fd0e03943c7572c4a7bc2a6770a5201926e0d1c6779e580553bc7cfffafd226b0db88be65e8e9f8a77f90ead631a96254c7ad8b6138976435cb6685e7dd5f567290ac6a4e6e4715cdd441418e1ec0c96cca970d2edc68c95b14e42a0bedb073038588c452fcc3ab85c5d1725a1a7880200a962e465e0f9d3f17fc3159f80fbfd30dc098cdc1a99737c44091712fdc9915499cb86525dca25f08198a7b402679d863eb2a02445fad7e28429afaf7c029fe6de81b785f1453e2f44c0c97c0618519c25c955c64156bc4ebe108f6d877fd532555f808b338826e1234c20bb2ccb22da3115fc75d93e41b0b21bd41532aafe2c5ac3ce6cc421cd2c4617aefb685fe0edeaa4938e6dd517820d09da9f3f01d8ede516dac6789e50a13567d2e439eeafdbfa2591a3ddfb128853087aae48a9d53e1d8fb48ee4515b37291704f31cf4d884035920a722325c47d404f63a5ab3833cc17c7117d088197ede501a1d2aa5e26cfbc4946734edf825a80c0bd829d71a6ff5be13ff2c21cb0e3dce66f73f7c30deae6c08738a0b6f231502620c55b44eeb77d256650ba7ade32188a7b72a1758cfc9b0df08e96db5728d2da080f494511bb845c10e66678a76337ebb3dd38980c827543059a159f7fdb62383d97cb2a8b89e16bbefd2111f7d67f0f396e10468e916e85c56b65222294520172052b927228118ade9c2a5345d38831c1ec55bb06534ee94ba43c072f7fa2303ac1d8973c436bb1c7b32bb904bb14c0bf00d8aaf28ff1c7f1f4cf8f7767e105c59c10c4daf99ddc0bcfb3cf4d124613dc9beeee7432d69312f3173edf7d31b1920e827a8ac303e56138695f31ea541b623e6b42cf3fc32635b806dc2f80c1a9c32580fe608a068ce6ad82d81aec14d4ff6e4289716e2d775764554fa24cb2e6766d5885115b9ba39aabcfe166368906efca5c804adecb21f7e84a9ba51b91cac472170ed426ab2407c18a25e5dd9dbdefaceed5249559e537100d9aad4df",
                "public_inputs" : [
                    3544317610574872,
                    3818006324670434,
                    1609100126983798,
                    2043470627881931,
                    194624568354568,
                    18446744069414584595
                ]
            }"#;

        match client.verify_merkle_root_from_json_string(proof_json).await {
            Ok(result) => {
                println!("Successfully verified : {result}")
            }
            Err(e) => {
                println!("Error verifying : {e:?}")
            }
        }
    }

    #[tokio::test]
    async fn verify_block_with_events() {
        let message_queue = "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9";
        let replayer = "0x5FC8d32690cc91D4c39d9d3abcBD16989F875707";
        let pk = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let url = "http://127.0.0.1:8545/";

        let client = ContractVerifiers::new(url, message_queue, replayer, Some(pk)).unwrap();

        let proof_json = r#"{
                "proof" : "18d39978105e6371129a8c670c4958719bf0b860646c2dd760a14c6b5aa04b8e1682aec235c07cc291c2bc14670ab30db45b6c6ce53e7d6e42d5d4837a6a0120183d34eb74c7afdf6d88b54e1bde6948e7f566f6cc374e8bec0ab5553e2b95392ecb009497004b9defb864e8756bbfc830dc0e1f505687c9c4779a32f6783943262140c77797264ea54462073603c736a6c78b20a3016f5493f5cf95556ee81e29ed533dc33499c78e45b8c3c36993a6ad812b7073d8f4ca1a61da68b44e28d00cca5e1481a5bf5fea36beae27af01d45bf45ae9d239fd0e03943c7572c4a7bc2a6770a5201926e0d1c6779e580553bc7cfffafd226b0db88be65e8e9f8a77f90ead631a96254c7ad8b6138976435cb6685e7dd5f567290ac6a4e6e4715cdd441418e1ec0c96cca970d2edc68c95b14e42a0bedb073038588c452fcc3ab85c5d1725a1a7880200a962e465e0f9d3f17fc3159f80fbfd30dc098cdc1a99737c44091712fdc9915499cb86525dca25f08198a7b402679d863eb2a02445fad7e28429afaf7c029fe6de81b785f1453e2f44c0c97c0618519c25c955c64156bc4ebe108f6d877fd532555f808b338826e1234c20bb2ccb22da3115fc75d93e41b0b21bd41532aafe2c5ac3ce6cc421cd2c4617aefb685fe0edeaa4938e6dd517820d09da9f3f01d8ede516dac6789e50a13567d2e439eeafdbfa2591a3ddfb128853087aae48a9d53e1d8fb48ee4515b37291704f31cf4d884035920a722325c47d404f63a5ab3833cc17c7117d088197ede501a1d2aa5e26cfbc4946734edf825a80c0bd829d71a6ff5be13ff2c21cb0e3dce66f73f7c30deae6c08738a0b6f231502620c55b44eeb77d256650ba7ade32188a7b72a1758cfc9b0df08e96db5728d2da080f494511bb845c10e66678a76337ebb3dd38980c827543059a159f7fdb62383d97cb2a8b89e16bbefd2111f7d67f0f396e10468e916e85c56b65222294520172052b927228118ade9c2a5345d38831c1ec55bb06534ee94ba43c072f7fa2303ac1d8973c436bb1c7b32bb904bb14c0bf00d8aaf28ff1c7f1f4cf8f7767e105c59c10c4daf99ddc0bcfb3cf4d124613dc9beeee7432d69312f3173edf7d31b1920e827a8ac303e56138695f31ea541b623e6b42cf3fc32635b806dc2f80c1a9c32580fe608a068ce6ad82d81aec14d4ff6e4289716e2d775764554fa24cb2e6766d5885115b9ba39aabcfe166368906efca5c804adecb21f7e84a9ba51b91cac472170ed426ab2407c18a25e5dd9dbdefaceed5249559e537100d9aad4df",
                "public_inputs" : [
                    3544317610574872,
                    3818006324670434,
                    1609100126983798,
                    2043470627881931,
                    194624568354568,
                    18446744069414584595
                ]
            }"#;

        match client.verify_merkle_root_from_json_string(proof_json).await {
            Ok(result) => {
                println!("Successfully verified : {result}")
            }
            Err(e) => {
                println!("Error verifying : {e:?}")
            }
        }

        let logs = client.fetch_merkle_roots(10000).await.unwrap();

        assert_ne!(logs.len(), 0);

        for (block, root, eth_tx) in logs.iter() {
            println!("- Block : {block} Merkle Root : {root} TxHash : {eth_tx}")
        }
    }

    #[tokio::test]
    async fn verify_merkle_proof() {
        let hash0 = H256::random();
        let hash1 = H256::random();
        let hash2 = H256::random();

        let leaf0 = Leaf::Hash(hash0);
        let leaf1 = Leaf::Hash(hash1);
        let leaf2 = Leaf::Hash(hash2);

        //let root = merkle_root::<_,_>(vec![leaf0, leaf1, leaf2]);
        let leaves = vec![hash0, hash1, hash2];

        let root = merkle_root::<Keccak256, _>(leaves.clone());
        let proof: MerkleProof<H256, H256> =
            merkle_proof::<Keccak256, Vec<H256>, H256>(leaves.clone(), 2);
        println!("leaves : {:?}", leaves);

        println!("Proof : {:?}", proof);
    }

    #[tokio::test]
    async fn verify_merkle_proof_for_msg() {
        let msg0 = ContentMessage {
            eth_address: Address::repeat_byte(1),
            vara_address: H256::repeat_byte(1),
            nonce: U256::from(1),
            data: Bytes::from(vec![1, 1]),
            //buf: Default::default(),
        };
        let msg1 = ContentMessage {
            eth_address: Address::repeat_byte(2),
            vara_address: H256::repeat_byte(2),
            nonce: U256::from(2),
            data: Bytes::from(vec![2, 2]),
            //buf: Default::default(),
        };
        let msg2 = ContentMessage {
            vara_address: H256::repeat_byte(4),
            eth_address: Address::repeat_byte(3),
            nonce: U256::from(3),
            data: Bytes::from(vec![3, 3]),
            //buf: Default::default(),
        };

        //let root = merkle_root::<_,_>(vec![leaf0, leaf1, leaf2]);
        let leaves = vec![msg0.to_bytes(), msg1.to_bytes(), msg2.to_bytes()];

        let root = merkle_root::<Keccak256, _>(leaves.clone());
        let proof: MerkleProof<H256, Vec<u8>> =
            merkle_proof::<Keccak256, _, Vec<u8>>(leaves.clone(), 2);

        let hash = keccak256(msg2.to_bytes());
        println!("Proof : {:?}", proof);
        println!("leaves : {:?}", leaves);
        println!("leaf hash: {:?}", hash);

        let is_ok = verify_proof::<Keccak256, _, _>(
            &proof.root,
            proof.proof,
            proof.number_of_leaves,
            proof.leaf_index,
            &proof.leaf,
        );

        println!("Ok: {:?}", is_ok);
    }

    #[tokio::test]
    async fn verify_merkle_proof_for_msgs() {
        let mut leaves: Vec<Vec<u8>> = Vec::new();

        for i in 0..100 {
            let msg = ContentMessage {
                eth_address: Address::repeat_byte(i as u8),
                vara_address: H256::repeat_byte(i as u8),
                nonce: U256::from(i as u8),
                data: Bytes::from(vec![i as u8, i as u8]),
            };
            leaves.push(msg.to_bytes())
        }

        let msg = ContentMessage {
            vara_address: H256::repeat_byte(7),
            eth_address: Address::repeat_byte(5),
            nonce: U256::from(10),
            data: Bytes::from(vec![3, 3, 3]),
        };
        leaves.push(msg.to_bytes());

        let root = merkle_root::<Keccak256, _>(leaves.clone());
        let proof: MerkleProof<H256, Vec<u8>> =
            merkle_proof::<Keccak256, _, Vec<u8>>(leaves.clone(), 100);

        let hash = keccak256(msg.to_bytes());
        println!("Proof : {:?}", proof);
        println!("leaves : {:?}", leaves);
        println!("leaf hash: {:?}", hash);

        let is_ok = verify_proof::<Keccak256, _, _>(
            &proof.root,
            proof.proof,
            proof.number_of_leaves,
            proof.leaf_index,
            &proof.leaf,
        );

        println!("Ok: {:?}", is_ok);
    }
}
