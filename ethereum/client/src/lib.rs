use alloy::{
    contract::Event,
    network::{Ethereum, EthereumWallet, TransactionBuilder},
    primitives::{Address, Bytes, B256, U256},
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            WalletFiller,
        },
        Identity, Provider, ProviderBuilder, RootProvider,
    },
    pubsub::Subscription,
    rpc::types::{Block, BlockId, BlockNumberOrTag, Filter, Header, Log as RpcLog},
    signers::local::PrivateKeySigner,
    sol_types::SolEvent,
    transports::{ws::WsConnect, RpcError, TransportErrorKind},
};
use anyhow::{Context, Result as AnyResult};
use primitive_types::{H160, H256};
use reqwest::Url;
use std::{ops::Deref, str::FromStr, time::Duration};

pub use alloy::primitives::TxHash;

pub mod abi;
use abi::{
    BridgingPayment, IERC20Manager, IMessageQueue,
    IMessageQueue::{IMessageQueueInstance, MerkleRoot, VaraMessage},
};

pub mod error;
pub use error::Error;

// 2 Gwei
const MAX_FEE_PER_GAS: u128 = 2_000_000_000;
// 0.5 Gwei
const MAX_PRIORITY_FEE_PER_GAS: u128 = 500_000_000;

type ProviderType = FillProvider<
    JoinFill<
        JoinFill<
            Identity,
            JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider<Ethereum>,
>;

#[derive(Clone)]
pub struct Contracts {
    provider: ProviderType,
    message_queue_instance: IMessageQueueInstance<ProviderType, Ethereum>,
}

#[derive(Debug, Clone)]
pub struct MerkleRootEntry {
    pub block_number: u64,
    pub merkle_root: H256,
}

#[derive(Debug, Clone)]
pub struct DepositEventEntry {
    pub from: H160,
    pub to: H256,
    pub token: H160,
    pub amount: primitive_types::U256,

    pub tx_hash: TxHash,
}

#[derive(Debug)]
pub enum TxStatus {
    Finalized,
    Pending,
    Failed,
}

#[derive(Clone)]
pub struct PollingEthApi {
    provider: RootProvider,
    url: Url,
}

impl PollingEthApi {
    pub async fn new(url: &str) -> AnyResult<Self> {
        let url = Url::parse(url).context("Provided url is not valid")?;
        let ws = WsConnect::new(url.clone());
        let provider = RootProvider::builder().connect_ws(ws).await?;

        Ok(Self { provider, url })
    }

    pub async fn reconnect(&self) -> AnyResult<Self> {
        let ws = WsConnect::new(self.url.clone());
        let provider = RootProvider::builder().connect_ws(ws).await?;

        Ok(Self {
            provider,
            url: self.url.clone(),
        })
    }

    pub async fn finalized_block(&self) -> AnyResult<Block> {
        self::finalized_block(&self.provider).await
    }

    pub async fn safe_block(&self) -> AnyResult<Block> {
        self::safe_block(&self.provider).await
    }

    pub async fn get_block(&self, block: u64) -> AnyResult<Block> {
        self::get_block(&self.provider, block).await
    }

    pub async fn fetch_fee_paid_events_txs(
        &self,
        contract_address: H160,
        block: u64,
    ) -> AnyResult<Vec<TxHash>> {
        let filter = Filter::new()
            .address(Address::from_slice(contract_address.as_bytes()))
            .event_signature(BridgingPayment::FeePaid::SIGNATURE_HASH)
            .from_block(block)
            .to_block(block);

        let event: Event<_, BridgingPayment::FeePaid, Ethereum> =
            Event::new(self.provider.clone(), filter);

        let logs = event.query().await.context("Failed to query event")?;

        logs.into_iter()
            .map(|(_, log)| log.transaction_hash.context("Failed to fetch transaction"))
            .collect()
    }

    pub async fn fetch_deposit_events(
        &self,
        contract_address: H160,
        block: u64,
    ) -> AnyResult<Vec<DepositEventEntry>> {
        let filter = Filter::new()
            .address(Address::from_slice(contract_address.as_bytes()))
            .event_signature(IERC20Manager::BridgingRequested::SIGNATURE_HASH)
            .from_block(block)
            .to_block(block);

        let event: Event<_, IERC20Manager::BridgingRequested, Ethereum> =
            Event::new(self.provider.clone(), filter);

        let logs = event.query().await.context("Failed to query event")?;

        logs.into_iter()
            .map(
                |(
                    IERC20Manager::BridgingRequested {
                        from,
                        to,
                        token,
                        amount,
                    },
                    log,
                )| {
                    let tx_hash = log
                        .transaction_hash
                        .context("Failed to fetch transaction")?;

                    Ok(DepositEventEntry {
                        from: H160(*from.0),
                        to: H256(to.0),
                        token: H160(*token.0),
                        amount: primitive_types::U256::from_little_endian(
                            &amount.to_le_bytes_vec(),
                        ),
                        tx_hash,
                    })
                },
            )
            .collect()
    }
}

impl Deref for PollingEthApi {
    type Target = RootProvider;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

pub async fn finalized_block(provider: impl Provider) -> AnyResult<Block> {
    provider
        .get_block_by_number(BlockNumberOrTag::Finalized)
        .await?
        .context("Finalized block is None")
}

pub async fn latest_block(provider: impl Provider) -> AnyResult<Block> {
    provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .context("Latest block is None")
}

pub async fn safe_block(provider: impl Provider) -> AnyResult<Block> {
    provider
        .get_block_by_number(BlockNumberOrTag::Safe)
        .await?
        .context("Safe block is None")
}

pub async fn get_block(provider: impl Provider, block: u64) -> AnyResult<Block> {
    provider
        .get_block_by_number(BlockNumberOrTag::Number(block))
        .await?
        .context("Block is None")
}

#[derive(Clone)]
pub struct EthApi {
    contracts: Contracts,
    public_key: Address,
    wallet: EthereumWallet,
    url: Url,
}

impl EthApi {
    pub async fn new(
        url: &str,
        message_queue_address: &str,
        private_key: Option<&str>,
    ) -> Result<EthApi, Error> {
        Self::new_with_retries(url, message_queue_address, private_key, None, None).await
    }

    pub async fn new_with_retries(
        url: &str,
        message_queue_address: &str,
        private_key: Option<&str>,
        ws_max_retry: Option<u32>,
        ws_retry_interval: Option<Duration>,
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

        let wallet = EthereumWallet::from(signer);

        let message_queue_address: Address = message_queue_address
            .parse()
            .map_err(|_| Error::WrongAddress)?;

        let url = Url::parse(url).map_err(|_| Error::WrongNodeUrl)?;

        let mut ws = WsConnect::new(url.clone());

        if let Some(ws_max_retry) = ws_max_retry {
            ws = ws.with_max_retries(ws_max_retry)
        }

        if let Some(ws_retry_interval) = ws_retry_interval {
            ws = ws.with_retry_interval(ws_retry_interval)
        }

        let provider: ProviderType = ProviderBuilder::new()
            .wallet(wallet.clone())
            .connect_ws(ws)
            .await?;

        let contracts = Contracts::new(provider, message_queue_address.into_array())?;

        Ok(EthApi {
            contracts,
            public_key,
            url,
            wallet,
        })
    }

    pub async fn reconnect(&self) -> Result<EthApi, Error> {
        let ws = WsConnect::new(self.url.clone());
        let provider: ProviderType = ProviderBuilder::new()
            .wallet(self.wallet.clone())
            .connect_ws(ws)
            .await?;

        let contracts = Contracts::new(
            provider,
            self.contracts.message_queue_instance.address().0 .0,
        )?;

        Ok(EthApi {
            contracts,
            public_key: self.public_key,
            url: self.url.clone(),
            wallet: self.wallet.clone(),
        })
    }

    // TODO: Don't expose provider here.
    pub fn raw_provider(&self) -> &ProviderType {
        &self.contracts.provider
    }

    /// Returns the maximum block number that can be submitted as part of a
    /// merkle-root submission into MessageQueue contract.
    pub async fn max_block_number(&self) -> Result<u32, Error> {
        self.contracts.max_block_number().await
    }

    /// Returns the delay (in seconds) requires before merkle-root submitted
    /// by an admin can be used.
    pub async fn process_admin_message_delay(&self) -> Result<u64, Error> {
        self.contracts.process_admin_message_delay().await
    }

    /// Returns the delay (in seconds) requires before merkle-root submitted
    /// by a pauser can be used.
    pub async fn process_pauser_message_delay(&self) -> Result<u64, Error> {
        self.contracts.process_pauser_message_delay().await
    }

    /// Returns the delay (in seconds) requires before merkle-root submitted
    /// by an arbitrary user can be used.
    pub async fn process_user_message_delay(&self) -> Result<u64, Error> {
        self.contracts.process_user_message_delay().await
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

    pub async fn send_challenge_root(&self) -> Result<TxHash, Error> {
        self.contracts.challenge_root().await
    }

    pub async fn get_tx_status(&self, tx_hash: TxHash) -> Result<TxStatus, Error> {
        self.contracts.get_tx_status(tx_hash).await
    }

    pub async fn read_finalized_merkle_root(
        &self,
        gear_block: u32,
    ) -> Result<Option<[u8; 32]>, Error> {
        self.contracts
            .read_merkle_root(U256::from(gear_block), BlockNumberOrTag::Finalized)
            .await
    }

    pub async fn read_chainhead_merkle_root(
        &self,
        gear_block: u32,
    ) -> Result<Option<[u8; 32]>, Error> {
        self.contracts
            .read_merkle_root(U256::from(gear_block), BlockNumberOrTag::Latest)
            .await
    }

    pub async fn fetch_merkle_roots_in_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<(MerkleRootEntry, Option<u64>)>, Error> {
        self.contracts.fetch_merkle_roots_in_range(from, to).await
    }

    pub async fn get_block_timestamp(&self, block: u64) -> Result<u64, Error> {
        Ok(self
            .raw_provider()
            .get_block_by_number(BlockNumberOrTag::Number(block))
            .await
            .map_err(Error::ErrorInHTTPTransport)?
            .ok_or(Error::ErrorFetchingBlock)?
            .header
            .timestamp)
    }

    pub async fn block_number(&self) -> Result<u64, Error> {
        self.contracts.block_number().await
    }

    pub async fn finalized_block_number(&self) -> AnyResult<u64> {
        Ok(self::finalized_block(self.raw_provider())
            .await?
            .header
            .number)
    }

    pub async fn latest_block_number(&self) -> AnyResult<u64> {
        Ok(self::latest_block(self.raw_provider()).await?.header.number)
    }

    pub async fn safe_block_number(&self) -> AnyResult<u64> {
        Ok(self::safe_block(self.raw_provider()).await?.header.number)
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
                U256::from_be_bytes(nonce),
                B256::from(sender),
                Address::from(receiver),
                Bytes::from(payload),
                proof.into_iter().map(B256::from).collect(),
            )
            .await
    }

    pub async fn is_message_processed(&self, nonce: [u8; 32]) -> Result<bool, Error> {
        self.contracts
            .is_message_processed(U256::from_be_bytes(nonce))
            .await
    }

    pub async fn subscribe_logs(
        &self,
    ) -> Result<Subscription<RpcLog>, RpcError<TransportErrorKind>> {
        let filter = Filter::new()
            .address(*self.contracts.message_queue_instance.address())
            .event_signature(IMessageQueue::MerkleRoot::SIGNATURE_HASH);

        self.raw_provider().clone().subscribe_logs(&filter).await
    }

    pub async fn subscribe_blocks(
        &self,
    ) -> Result<Subscription<Header>, RpcError<TransportErrorKind>> {
        self.raw_provider().clone().subscribe_blocks().await
    }
}

impl Contracts {
    pub fn new(provider: ProviderType, message_queue_address: [u8; 20]) -> Result<Self, Error> {
        let message_queue_address = Address::from(message_queue_address);
        let message_queue_instance = IMessageQueue::new(message_queue_address, provider.clone());

        Ok(Contracts {
            provider,
            message_queue_instance,
        })
    }

    pub async fn max_block_number(&self) -> Result<u32, Error> {
        self.message_queue_instance
            .maxBlockNumber()
            .call()
            .await
            .map(|num| num.to())
            .map_err(Error::ErrorDuringContractExecution)
    }

    pub async fn process_admin_message_delay(&self) -> Result<u64, Error> {
        self.message_queue_instance
            .PROCESS_ADMIN_MESSAGE_DELAY()
            .call()
            .await
            .map(|delay| delay.to())
            .map_err(Error::ErrorDuringContractExecution)
    }

    pub async fn process_pauser_message_delay(&self) -> Result<u64, Error> {
        self.message_queue_instance
            .PROCESS_PAUSER_MESSAGE_DELAY()
            .call()
            .await
            .map(|delay| delay.to())
            .map_err(Error::ErrorDuringContractExecution)
    }

    pub async fn process_user_message_delay(&self) -> Result<u64, Error> {
        self.message_queue_instance
            .PROCESS_USER_MESSAGE_DELAY()
            .call()
            .await
            .map(|delay| delay.to())
            .map_err(Error::ErrorDuringContractExecution)
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
            .message_queue_instance
            .submitMerkleRoot(block_number, merkle_root, proof.clone())
            .estimate_gas()
            .await
        {
            Ok(gas_used) => {
                log::info!("Gas used: {gas_used}");
                match self
                    .message_queue_instance
                    .submitMerkleRoot(block_number, merkle_root, proof.clone())
                    .send()
                    .await
                {
                    Ok(pending_tx) => Ok(*pending_tx.tx_hash()),
                    Err(e) => {
                        log::error!("Sending error: {e:?}");
                        if let Some(e) =
                            e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                        {
                            return Err(Error::MessageQueue(e));
                        }

                        Err(Error::ErrorSendingTransaction(e))
                    }
                }
            }

            Err(e) => {
                if let Some(e) =
                    e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                {
                    return Err(Error::MessageQueue(e));
                }

                Err(Error::ErrorDuringContractExecution(e))
            }
        }
    }

    pub async fn challenge_root(&self) -> Result<TxHash, Error> {
        match self
            .message_queue_instance
            .challengeRoot()
            .estimate_gas()
            .await
        {
            Ok(gas_used) => {
                log::info!("Gas used: {gas_used}");
                match self.message_queue_instance.challengeRoot().send().await {
                    Ok(pending_tx) => Ok(*pending_tx.tx_hash()),
                    Err(e) => {
                        log::error!("Sending error: {e:?}");
                        if let Some(e) =
                            e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                        {
                            return Err(Error::MessageQueue(e));
                        }

                        Err(Error::ErrorSendingTransaction(e))
                    }
                }
            }

            Err(e) => {
                if let Some(e) =
                    e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                {
                    return Err(Error::MessageQueue(e));
                }

                Err(Error::ErrorDuringContractExecution(e))
            }
        }
    }

    pub async fn block_number(&self) -> Result<u64, Error> {
        self.provider.get_block_number().await.map_err(|e| e.into())
    }

    pub async fn fetch_merkle_roots(
        &self,
        depth: u64,
    ) -> Result<Vec<(MerkleRootEntry, Option<u64>)>, Error> {
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
    ) -> Result<Vec<(MerkleRootEntry, Option<u64>)>, Error> {
        let filter = Filter::new()
            .address(*self.message_queue_instance.address())
            .event_signature(IMessageQueue::MerkleRoot::SIGNATURE_HASH)
            .from_block(from)
            .to_block(to);

        let event: Event<ProviderType, MerkleRoot, Ethereum> =
            Event::new(self.provider.clone(), filter);

        let logs = event.query().await.map_err(Error::ErrorQueryingEvent)?;

        Ok(logs
            .iter()
            .map(|(event, log)| {
                (
                    MerkleRootEntry {
                        block_number: event.blockNumber.to(),
                        merkle_root: event.merkleRoot.0.into(),
                    },
                    log.block_number,
                )
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn provide_content_message(
        &self,
        block_number: U256,
        total_leaves: U256,
        leaf_index: U256,
        nonce: U256,
        source: B256,
        destination: Address,
        payload: Bytes,
        proof: Vec<B256>,
    ) -> Result<TxHash, Error> {
        log::trace!(
            "provide_content_message: block_number = {block_number}, total_leaves = {total_leaves}, leaf_index = {leaf_index}, nonce = {nonce}, source = {source}, destination = {destination}, payload = {payload}, proof = {proof:?}",
        );

        let call = self.message_queue_instance.processMessage(
            block_number,
            total_leaves,
            leaf_index,
            VaraMessage {
                nonce,
                source,
                destination,
                payload,
            },
            proof,
        );

        let gas_estimated = match call.estimate_gas().await {
            Ok(gas_estimated) => gas_estimated,
            Err(e) => {
                if let Some(e) =
                    e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                {
                    return Err(Error::MessageQueue(e));
                }

                return Err(Error::ErrorDuringContractExecution(e));
            }
        };

        let max_priority_fee_per_gas = self.provider.get_max_priority_fee_per_gas().await;
        let gas_price = self.provider.get_gas_price().await;
        log::trace!("max_priority_fee_per_gas_chain = {max_priority_fee_per_gas:?}, gas_price_chain = {gas_price:?}");

        let max_fee_per_gas = gas_price
            .map(|gas_price| {
                if gas_price < MAX_FEE_PER_GAS {
                    MAX_FEE_PER_GAS
                } else {
                    gas_price
                }
            })
            .unwrap_or(MAX_FEE_PER_GAS);

        let max_priority_fee_per_gas = max_priority_fee_per_gas
            .map(|max_priority_fee_per_gas| {
                if max_priority_fee_per_gas < MAX_PRIORITY_FEE_PER_GAS {
                    MAX_PRIORITY_FEE_PER_GAS
                } else {
                    max_priority_fee_per_gas
                }
            })
            .unwrap_or(MAX_PRIORITY_FEE_PER_GAS);

        let call = call
            .max_fee_per_gas(max_fee_per_gas)
            .max_priority_fee_per_gas(max_priority_fee_per_gas);

        let request = call.as_ref();
        log::trace!(
            "new max_priority_fee_per_gas = {max_priority_fee_per_gas:?}, new max_fee_per_gas = {max_fee_per_gas:?}, gas_estimated = {gas_estimated}, gas_price = {:?}, max_fee_per_gas = {:?}, max_priority_fee_per_gas = {:?}, gas_limit = {:?}",
            request.gas_price(),
            request.max_fee_per_gas(),
            request.max_priority_fee_per_gas(),
            request.gas_limit(),
        );

        match call.send().await {
            Ok(pending_tx) => Ok(*pending_tx.tx_hash()),
            Err(e) => {
                log::error!("Sending error: {e:?}");
                if let Some(e) =
                    e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                {
                    return Err(Error::MessageQueue(e));
                }

                Err(Error::ErrorSendingTransaction(e))
            }
        }
    }

    pub async fn read_merkle_root(
        &self,
        block: U256,
        block_tag: BlockNumberOrTag,
    ) -> Result<Option<[u8; 32]>, Error> {
        let root = self
            .message_queue_instance
            .getMerkleRoot(block)
            .block(BlockId::Number(block_tag))
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?
            .0;

        Ok((root != [0; 32]).then_some(root))
    }

    pub async fn is_message_processed(&self, nonce: U256) -> Result<bool, Error> {
        let processed = self
            .message_queue_instance
            .isProcessed(nonce)
            .block(BlockId::Number(BlockNumberOrTag::Finalized))
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?;

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
            .get_block_by_number(BlockNumberOrTag::Finalized)
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
