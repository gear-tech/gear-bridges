use std::{marker::PhantomData, str::FromStr};

use abi::IRelayer::MerkleRoot;

use alloy::{
    contract::Event,
    network::{Ethereum, EthereumWallet},
    primitives::{Address, Bytes, B256, U256},
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            WalletFiller,
        },
        Identity, Provider, ProviderBuilder, RootProvider,
    },
    rpc::types::{BlockId, BlockNumberOrTag, Filter},
    signers::local::PrivateKeySigner,
    sol_types::SolEvent,
    transports::{
        http::{Client, Http},
        Transport,
    },
};

use reqwest::Url;

pub use error::Error;

use crate::abi::{
    ContentMessage, IMessageQueue, IMessageQueue::IMessageQueueInstance, IRelayer,
    IRelayer::IRelayerInstance,
};
pub use alloy::primitives::TxHash;

#[cfg(test)]
mod tests;

mod abi;
pub mod error;

type ProviderType = FillProvider<
    JoinFill<
        JoinFill<
            Identity,
            JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider<Http<Client>>,
    Http<Client>,
    Ethereum,
>;

#[derive(Clone)]
pub struct Contracts<P, T, N> {
    provider: P,
    message_queue_instance: IMessageQueueInstance<T, P, N>,
    relayer_instance: IRelayerInstance<T, P, N>,
    _phantom: PhantomData<(T, N)>,
}

#[derive(Debug, Clone)]
pub struct MerkleRootEntry {
    pub block_number: u64,
}

#[derive(Debug)]
pub enum TxStatus {
    Finalized,
    Pending,
    Failed,
}

#[derive(Clone)]
pub struct EthApi {
    contracts: Contracts<ProviderType, Http<Client>, Ethereum>,
    public_key: Address,
}

impl EthApi {
    pub fn new(
        url: &str,
        message_queue_address: &str,
        relayer_address: &str,
        private_key: Option<&str>,
    ) -> Result<EthApi, Error> {
        let signer = match private_key {
            Some(private_key) => {
                let pk: B256 =
                    B256::from(U256::from_str(private_key).map_err(|_| Error::WrongPrivateKey)?);
                PrivateKeySigner::from_bytes(&pk).map_err(|_| Error::WrongPrivateKey)?
            }
            None => PrivateKeySigner::random(),
        };

        let public_key = signer.address();

        let wallet = alloy::network::EthereumWallet::from(signer);

        let message_queue_address: Address = message_queue_address
            .parse()
            .map_err(|_| Error::WrongAddress)?;
        let relayer_address: Address = relayer_address.parse().map_err(|_| Error::WrongAddress)?;

        let url = Url::parse(url).map_err(|_| Error::WrongNodeUrl)?;

        let provider: ProviderType = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet)
            .on_http(url);

        let contracts = Contracts::new(
            provider,
            message_queue_address.into_array(),
            relayer_address.into_array(),
        )?;

        Ok(EthApi {
            contracts,
            public_key,
        })
    }

    pub async fn get_approx_balance(&self) -> Result<f64, Error> {
        self.contracts.get_approx_balance(self.public_key).await
    }

    pub async fn provide_merkle_root(
        &self,
        block_number: u32,
        merkle_root: [u8; 32],
        proof: Vec<u8>,
    ) -> Result<TxHash, Error> {
        self.contracts
            .provide_merkle_root(
                U256::from(block_number),
                B256::from(merkle_root),
                Bytes::from(proof),
            )
            .await
    }

    pub async fn get_tx_status(&self, tx_hash: TxHash) -> Result<TxStatus, Error> {
        self.contracts.get_tx_status(tx_hash).await
    }

    pub async fn read_finalized_merkle_root(&self, block: u32) -> Result<Option<[u8; 32]>, Error> {
        self.contracts
            .read_finalized_merkle_root(U256::from(block))
            .await
    }

    pub async fn fetch_merkle_roots_in_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<MerkleRootEntry>, Error> {
        self.contracts.fetch_merkle_roots_in_range(from, to).await
    }

    pub async fn block_number(&self) -> Result<u64, Error> {
        self.contracts.block_number().await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn provide_content_message(
        &self,
        block_number: u32,
        total_leaves: u32,
        leaf_index: u32,
        nonce: [u8; 32],
        sender: [u8; 32],
        receiver: [u8; 20],
        payload: Vec<u8>,
        proof: Vec<[u8; 32]>,
    ) -> Result<TxHash, Error> {
        self.contracts
            .provide_content_message(
                U256::from(block_number),
                U256::from(total_leaves),
                U256::from(leaf_index),
                B256::from(nonce),
                B256::from(sender),
                Address::from(receiver),
                Bytes::from(payload),
                proof.into_iter().map(B256::from).collect(),
            )
            .await
    }

    pub async fn is_message_processed(&self, nonce: [u8; 32]) -> Result<bool, Error> {
        self.contracts.is_message_processed(B256::from(nonce)).await
    }
}

impl<P, T> Contracts<P, T, Ethereum>
where
    T: Transport + Clone,
    P: Provider<T, Ethereum> + Send + Sync + Clone + 'static,
{
    pub fn new(
        provider: P,
        message_queue_address: [u8; 20],
        relayer_address: [u8; 20],
    ) -> Result<Self, Error> {
        let relayer_address = Address::from(relayer_address);
        let message_queue_address = Address::from(message_queue_address);

        let relayer_instance = IRelayer::new(relayer_address, provider.clone());
        let message_queue_instance = IMessageQueue::new(message_queue_address, provider.clone());

        Ok(Contracts {
            provider,
            relayer_instance,
            message_queue_instance,
            _phantom: PhantomData,
        })
    }

    pub async fn get_approx_balance(&self, address: Address) -> Result<f64, Error> {
        let balance = self.provider.get_balance(address).latest().await?;
        let balance: f64 = balance.into();
        Ok(balance / 1_000_000_000_000_000_000.0)
    }

    pub async fn provide_merkle_root(
        &self,
        block_number: U256,
        merkle_root: B256,
        proof: Bytes,
    ) -> Result<TxHash, Error> {
        match self
            .relayer_instance
            .submitMerkleRoot(block_number, merkle_root, proof.clone())
            .estimate_gas()
            .await
        {
            Ok(gas_used) => {
                log::info!("Gas used: {gas_used}");
                match self
                    .relayer_instance
                    .submitMerkleRoot(block_number, merkle_root, proof.clone())
                    .send()
                    .await
                {
                    Ok(pending_tx) => Ok(*pending_tx.tx_hash()),
                    Err(e) => {
                        log::error!("Sending error: {e:?}");
                        Err(Error::ErrorSendingTransaction(e))
                    }
                }
            }
            Err(e) => Err(Error::ErrorDuringContractExecution(e)),
        }
    }

    pub async fn block_number(&self) -> Result<u64, Error> {
        self.provider.get_block_number().await.map_err(|e| e.into())
    }

    pub async fn fetch_merkle_roots(&self, depth: u64) -> Result<Vec<MerkleRootEntry>, Error> {
        let current_block: u64 = self.provider.get_block_number().await?;

        self.fetch_merkle_roots_in_range(
            current_block.checked_sub(depth).unwrap_or_default(),
            current_block,
        )
        .await
    }

    pub async fn fetch_merkle_roots_in_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<MerkleRootEntry>, Error> {
        let filter = Filter::new()
            .address(*self.relayer_instance.address())
            .event_signature(IRelayer::MerkleRoot::SIGNATURE_HASH)
            .from_block(from)
            .to_block(to);

        let event: Event<T, P, MerkleRoot, Ethereum> = Event::new(self.provider.clone(), filter);

        let logs = event.query().await.map_err(Error::ErrorQueryingEvent)?;

        Ok(logs
            .iter()
            .map(|(event, _)| MerkleRootEntry {
                block_number: event.blockNumber.to(),
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn provide_content_message(
        &self,
        block_number: U256,
        total_leaves: U256,
        leaf_index: U256,
        nonce: B256,
        sender: B256,
        receiver: Address,
        data: Bytes,
        proof: Vec<B256>,
    ) -> Result<TxHash, Error> {
        let call = self.message_queue_instance.processMessage(
            block_number,
            total_leaves,
            leaf_index,
            ContentMessage {
                nonce,
                sender,
                receiver,
                data,
            },
            proof,
        );

        match call.estimate_gas().await {
            Ok(_gas_used) => match call.send().await {
                Ok(pending_tx) => Ok(*pending_tx.tx_hash()),
                Err(e) => {
                    log::error!("Sending error: {e:?}");
                    Err(Error::ErrorSendingTransaction(e))
                }
            },
            Err(e) => Err(Error::ErrorDuringContractExecution(e)),
        }
    }

    pub async fn read_finalized_merkle_root(&self, block: U256) -> Result<Option<[u8; 32]>, Error> {
        let root = self
            .relayer_instance
            .getMerkleRoot(block)
            .block(BlockId::Number(BlockNumberOrTag::Finalized))
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?
            ._0
            .0;

        Ok((root != [0; 32]).then_some(root))
    }

    pub async fn is_message_processed(&self, nonce: B256) -> Result<bool, Error> {
        // TODO: Change isProcessed to accept only nonce.
        let processed = self
            .message_queue_instance
            .isProcessed(ContentMessage {
                nonce,
                sender: Default::default(),
                receiver: Default::default(),
                data: Default::default(),
            })
            .block(BlockId::Number(BlockNumberOrTag::Finalized))
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?
            ._0;

        Ok(processed)
    }

    pub async fn get_tx_status(&self, tx_hash: TxHash) -> Result<TxStatus, Error> {
        let tx = self
            .provider
            .get_transaction_by_hash(tx_hash)
            .await
            .map_err(|_| Error::ErrorFetchingTransaction)?
            .ok_or(Error::ErrorFetchingTransaction)?;

        if tx.block_hash.is_none() {
            return Ok(TxStatus::Pending);
        }

        let receipt = self
            .provider
            .get_transaction_receipt(tx_hash)
            .await
            .map_err(|_| Error::ErrorFetchingTransactionReceipt)?;

        let receipt = if let Some(receipt) = receipt {
            receipt
        } else {
            return Ok(TxStatus::Failed);
        };

        let tx_status = receipt.status();

        if !tx_status {
            return Ok(TxStatus::Failed);
        }

        let block = if let Some(block) = receipt.block_number {
            block
        } else {
            return Ok(TxStatus::Pending);
        };

        let latest_finalized = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Finalized, false)
            .await
            .map_err(|_| Error::ErrorFetchingBlock)?
            .ok_or(Error::ErrorFetchingBlock)?
            .header
            .number;

        let status = if latest_finalized >= block {
            TxStatus::Finalized
        } else {
            TxStatus::Pending
        };

        Ok(status)
    }
}
