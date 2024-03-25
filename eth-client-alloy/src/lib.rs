pub mod error;
mod abi;
mod convert;
mod proof;

use error::VerifierError;
use serde::Deserialize;
use std::sync::Arc;
use alloy_network::{Ethereum, EthereumSigner, Network, TxSigner};
use alloy_provider::{Provider, ProviderBuilder, ProviderLayer, RootProvider};
use alloy_transport::{BoxTransport, Transport};
use alloy_rpc_client::RpcClient;
use reqwest::{Client, Url};
use alloy_primitives::{Address, B256, Bytes, hex, U256};
use alloy_provider::layers::{GasEstimatorLayer, GasEstimatorProvider, ManagedNonceLayer, ManagedNonceProvider, SignerProvider};
use alloy_signer_wallet::{LocalWallet, Wallet};
use alloy_sol_types::{sol, SolCall, SolInterface};
use crate::abi::IRelayer::IRelayerInstance;
use crate::abi::IMessageQueue::IMessageQueueInstance;
use crate::abi::{IMessageQueue, IRelayer};
use crate::convert::Convert;
use crate::proof::Proof;
use std::marker::PhantomData;
use alloy_rpc_types::TransactionRequest;
use alloy_signer::k256::ecdsa::SigningKey;


type ProviderType = ManagedNonceProvider<Ethereum, BoxTransport, GasEstimatorProvider<Ethereum, BoxTransport, SignerProvider<Ethereum, BoxTransport, RootProvider<Ethereum, BoxTransport>, EthereumSigner>>>;


    pub struct ContractVerifiers
    {
        signer : Wallet<SigningKey>,
        provider : Arc<ProviderType>,
        message_queue_instance : IMessageQueueInstance<Ethereum, BoxTransport , Arc<ProviderType>>,
        relayer_instance : IRelayerInstance<Ethereum, BoxTransport , Arc<ProviderType>>,
    }


    impl ContractVerifiers
    {
        pub fn new(
            url : &str,
            message_queue_address_str: &str,
            relayer_address_str: &str,
            pk_str: Option<&str>
        ) -> Result<Self, VerifierError> {

            let http = alloy_transport_http::Http::<Client>::new(Url::parse(url).unwrap()).boxed();
            let rp = RootProvider::<Ethereum, BoxTransport>::new(RpcClient::new(http, true));

            let signer = match pk_str {
                Some(pk) => {
                    let pk = hex::decode(pk).map_err(|_| VerifierError::WrongPrivateKey)?;
                    LocalWallet::from_bytes(&B256::from_slice(pk.as_slice())).map_err(|_| VerifierError::WrongPrivateKey)?
                }
                None => {
                    LocalWallet::random()
                }
            };


            let provider  = Arc::new(ProviderBuilder::new()
                .layer(ManagedNonceLayer)
                .layer(GasEstimatorLayer)
                .signer(EthereumSigner::from(signer.clone())) // note the order!
                .provider(rp));

            let relayer_contract_address : Address = relayer_address_str.parse().map_err(|_| VerifierError::WrongAddress)?;
            let message_queue_contract_address : Address = message_queue_address_str.parse().map_err(|_| VerifierError::WrongAddress)?;


            let relayer_instance = IRelayer::new(relayer_contract_address, provider.clone());
            let message_queue_instance = IMessageQueue::new(message_queue_contract_address, provider.clone());

            Ok(ContractVerifiers{
                signer,
                provider,
                relayer_instance,
                message_queue_instance,
            })


        }


        pub async fn verify_merkle_root_tx<U : Convert<U256>, B : Convert<Bytes>>(&self, public_inputs : Vec<U>, proof : B  )->Result<bool, VerifierError>{
            let public_inputs : Vec<U256> = public_inputs.into_iter().map(|v|v.convert()).collect();
            let proof = proof.convert();

            match self.relayer_instance.submit_merkle_root(public_inputs.clone(), proof.clone())
                .estimate_gas().await {
                Ok(gas_used) => {
                    println!("Gas used: {gas_used}");
                    match self.relayer_instance.submit_merkle_root(public_inputs, proof).from(self.signer.address()).gas_price(U256::from(2_000_000_000_000u128)).send().await {
                        Ok(pending_tx)=>{
                            match pending_tx.get_receipt().await {
                                Ok(receipt)=>{
                                    Ok(true)
                                }
                                Err(e)=>{
                                    Err(VerifierError::ErrorWaitingTransactionReceipt)
                                }
                            }
                        }
                        Err(e)=>{
                            println!("Sending error: {e:?}");
                            Err(VerifierError::ErrorSendingTransaction)
                        }
                    }
                }
                Err(e)=>{
                    Err(VerifierError::ErrorDuringContractExecution)
                }
            }

        }
        pub async fn verify_merkle_root_from_json_string(
            &self,
            json_string: &str,
        ) -> Result<bool, VerifierError> {
            let proof : Proof = Proof::try_from_json_string(json_string).map_err(|_|VerifierError::WrongJsonFormation)?;
            self.verify_merkle_root_tx(proof.public_inputs, proof.proof).await
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
        use crate::*;

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
                Ok(result)=>{
                    println!("Successfully verified : {result}")
                }
                Err(e)=>{
                    println!("Error verifying : {e:?}")
                }
            }

        }

    }
