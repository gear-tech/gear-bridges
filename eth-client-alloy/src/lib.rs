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
use alloy_provider::layers::{ManagedNonceLayer, ManagedNonceProvider, SignerProvider};
use alloy_signer_wallet::LocalWallet;
use alloy_sol_types::{sol, SolCall, SolInterface};
use crate::abi::IRelayer::IRelayerInstance;
use crate::abi::IMessageQueue::IMessageQueueInstance;
use crate::abi::{IMessageQueue, IRelayer};
use crate::convert::Convert;
use crate::proof::Proof;
use std::marker::PhantomData;
use alloy_rpc_types::TransactionRequest;


type ProviderType = ManagedNonceProvider<Ethereum, BoxTransport, SignerProvider<Ethereum, BoxTransport, RootProvider<Ethereum, BoxTransport>, EthereumSigner>>;


    pub struct ContractVerifiers
    {
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
                .signer(EthereumSigner::from(signer)) // note the order!
                .provider(rp));

            let relayer_contract_address : Address = relayer_address_str.parse().map_err(|_| VerifierError::WrongAddress)?;
            let message_queue_contract_address : Address = message_queue_address_str.parse().map_err(|_| VerifierError::WrongAddress)?;


            let relayer_instance = IRelayer::new(relayer_contract_address, provider.clone());
            let message_queue_instance = IMessageQueue::new(message_queue_contract_address, provider.clone());

            Ok(ContractVerifiers{
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
                    match self.relayer_instance.submit_merkle_root(public_inputs, proof).send().await {
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
        async fn validator_set_change_verifier() {
            let vs_change_vrf_address = "5FbDB2315678afecb367f032d93F642f64180aa3";
            let msg_sent_vrf_address = "e7f1725E7734CE288F8367e1Bb143E90bb3F0512";
            let url = "http://127.0.0.1:8545/";
            let client = ContractVerifiers::new(url, vs_change_vrf_address, msg_sent_vrf_address, None).unwrap();
            //let verifier = ContractVerifiers::new(url, vs_change_vrf_address, msg_sent_vrf_address)
            //    .expect("Error during verifiers instantiation");

            /*println!(
                "{:?}",
                verifier
                    .verify_vs_change(
                        String::from(
                            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
                        ),
                        "../solidity_verifier/aggregation/vs_change/final_proof.json",
                        "../solidity_verifier/aggregation/vs_change/final_public.json"
                    )
                    .await
            );

            println!("{:?}", verifier.get_all_validator_sets_from_vs_vrf().await);

            println!("{:?}", verifier.get_last_validator_set_from_vs_vrf().await);

            println!("{:?}", verifier.get_nonce_id_from_vs_vrf().await);

            panic!();
             */
        }

        // uncomment when there will be public inputs for msg sent verifier

        // #[tokio::test]
        // async fn msg_sent_verifier() {
        //     let vs_change_vrf_address = "5FbDB2315678afecb367f032d93F642f64180aa3";
        //     let msg_sent_vrf_address = "e7f1725E7734CE288F8367e1Bb143E90bb3F0512";
        //     let url = "http://127.0.0.1:8545/";
        //     let verifier = ContractVerifiers::new(url, vs_change_vrf_address, msg_sent_vrf_address)
        //         .expect("Error during verifiers instantiation");

        //     println!(
        //         "{:?}",
        //         verifier
        //             .verify_msg_sent(
        //                 String::from("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"),
        //                 "../solidity_verifier/aggregation/message_sent/final_proof.json",
        //                 "../solidity_verifier/aggregation/message_sent/final_public.json"
        //             )
        //             .await
        //     );

        //     println!(
        //         "{:?}",
        //         verifier
        //             .get_all_msg_hashes_from_msg_sent_vrf()
        //             .await
        //     );

        //     println!(
        //         "{:?}",
        //         verifier
        //             .get_last_msg_hashes_from_msg_sent_vrf()
        //             .await
        //     );
        // }


    }
