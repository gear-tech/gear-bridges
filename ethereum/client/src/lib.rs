use std::{marker::PhantomData, str::FromStr};

use abi::IRelayer::MerkleRoot;

use alloy::{
    contract::Event,
    network::{Ethereum, Network},
    primitives::{Address, Bytes, B256, U256},
    providers::{
        fillers::{GasFiller, NonceFiller, WalletFiller},
        Provider, ProviderBuilder, RootProvider,
    },
    rpc::types::{BlockId, BlockNumberOrTag, Filter},
    signers::local::PrivateKeySigner,
    sol_types::SolEvent,
    transports::{
        http::{Client, Http},
        BoxTransport, Transport,
    },
};

use reqwest::Url;

use error::Error;

use crate::{
    abi::{
        ContentMessage, IMessageQueue, IMessageQueue::IMessageQueueInstance, IRelayer,
        IRelayer::IRelayerInstance,
    },
    convert::Convert,
    proof::BlockMerkleRootProof,
};

mod abi;
mod convert;
pub mod error;
mod proof;

type ProviderType = alloy::providers::fillers::FillProvider<
    alloy::providers::fillers::JoinFill<
        alloy::providers::fillers::JoinFill<
            alloy::providers::fillers::JoinFill<
                alloy::providers::fillers::JoinFill<alloy::providers::Identity, GasFiller>,
                NonceFiller,
            >,
            alloy::providers::fillers::ChainIdFiller,
        >,
        WalletFiller<alloy::network::EthereumWallet>,
    >,
    RootProvider<alloy::transports::http::Http<alloy::transports::http::Client>>,
    alloy::transports::http::Http<alloy::transports::http::Client>,
    Ethereum,
>;

pub use alloy::primitives::TxHash;

#[derive(Clone)]
pub struct Contracts<P, T, N> {
    //signer: PrivateKeySigner,
    provider: P,
    message_queue_instance: IMessageQueueInstance<T, P, N>,
    relayer_instance: IRelayerInstance<T, P, N>,
    _t: PhantomData<T>,
    _n: PhantomData<N>,
}

#[allow(dead_code)]
pub struct MerkleRootEntry {
    pub block_number: u64,
    merkle_root: B256,
    tx_hash: TxHash,
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
}

impl EthApi {
    pub fn new(
        url: &String,
        message_queue_address: &String,
        relayer_address: &String,
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

        Ok(EthApi { contracts })
    }

    pub async fn provide_merkle_root<U: Convert<U256>, H: Convert<B256>, B: Convert<Bytes>>(
        &self,
        block_number: U,
        merkle_root: H,
        proof: B,
    ) -> Result<TxHash, Error> {
        Err(Error::NotImplemented)
    }

    pub async fn get_tx_status(&self, tx_hash: TxHash) -> Result<TxStatus, Error> {
        self.contracts.get_tx_status(tx_hash).await
    }

    pub async fn read_finalized_merkle_root(&self, block: u32) -> Result<Option<[u8; 32]>, Error> {
        self.contracts.read_finalized_merkle_root(block).await
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
                block_number as u64,
                total_leaves as u64,
                leaf_index as u64,
                nonce,
                sender,
                receiver,
                payload,
                proof,
            )
            .await
    }

    pub async fn is_message_processed(&self, nonce_le: [u8; 32]) -> Result<bool, Error> {
        self.contracts.is_message_processed(nonce_le).await
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
            _n: PhantomData,
            _t: PhantomData,
        })
    }

    pub async fn provide_merkle_root<U: Convert<U256>, H: Convert<B256>, B: Convert<Bytes>>(
        &self,
        block_number: U,
        merkle_root: H,
        proof: B,
    ) -> Result<TxHash, Error> {
        let block_number: U256 = block_number.convert();
        let merkle_root: B256 = merkle_root.convert();
        let proof = proof.convert();

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

    pub async fn provide_merkle_root_json(&self, json_string: &str) -> Result<B256, Error> {
        let proof: BlockMerkleRootProof = BlockMerkleRootProof::try_from_json_string(json_string)
            .map_err(|_| Error::WrongJsonFormation)?;

        self.provide_merkle_root(proof.block_number, proof.merkle_root, proof.proof)
            .await
    }

    pub async fn block_number(&self) -> Result<u64, Error> {
        self.provider
            .get_block_number()
            .await
            .map_err(|_| Error::ErrorInHTTPTransport)
    }

    pub async fn fetch_merkle_roots(&self, depth: u64) -> Result<Vec<MerkleRootEntry>, Error> {
        let current_block: u64 = self
            .provider
            .get_block_number()
            .await
            .map_err(|_| Error::ErrorInHTTPTransport)?;

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

        match event.query().await {
            Ok(logs) => Ok(logs
                .iter()
                .map(|(event, log)| MerkleRootEntry {
                    block_number: event.blockNumber.to(),
                    merkle_root: event.merkleRoot,
                    tx_hash: log.transaction_hash.unwrap_or_default(),
                })
                .collect()),
            Err(_) => Err(Error::ErrorInHTTPTransport),
        }
    }

    pub async fn provide_content_message(
        &self,
        block_number: u64,
        total_leaves: u64,
        leaf_index: u64,
        nonce: [u8; 32],
        sender: [u8; 32],
        receiver: [u8; 20],
        payload: Vec<u8>,
        proof: Vec<[u8; 32]>,
    ) -> Result<TxHash, Error> {
        let call = self.message_queue_instance.processMessage(
            U256::from(block_number),
            U256::from(total_leaves),
            U256::from(leaf_index),
            ContentMessage {
                nonce: B256::from(nonce),
                sender: B256::from(sender),
                receiver: Address::from(receiver),
                data: Bytes::from(payload),
            },
            proof.into_iter().map(|x| B256::from(x)).collect(),
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

    pub async fn read_finalized_merkle_root(&self, block: u32) -> Result<Option<[u8; 32]>, Error> {
        let block = U256::from(block);

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

    pub async fn is_message_processed(&self, nonce_le: [u8; 32]) -> Result<bool, Error> {
        let nonce = B256::from(nonce_le);

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
            .number
            .ok_or(Error::ErrorFetchingBlock)?;

        let status = if latest_finalized >= block {
            TxStatus::Finalized
        } else {
            TxStatus::Pending
        };

        Ok(status)
    }
}

#[cfg(test)]
mod tests {

    use alloy::hex;
    use alloy::node_bindings::{Anvil, AnvilInstance};
    use alloy::primitives::keccak256;
    use alloy::providers::ProviderBuilder;
    use alloy::transports::http::{Client, Http};
    use binary_merkle_tree::{merkle_proof, merkle_root, verify_proof, Leaf, MerkleProof};
    use primitive_types::H256;
    use sp_core::KeccakHasher;

    use crate::abi::{
        ERC20Mock, ERC20Treasury, MessageQueue, ProxyContract, Relayer, Verifier, VerifierMock,
    };

    use super::*;

    fn build_contracts<P, T>(provider: P) -> Result<Contracts<P, T, Ethereum>, Error>
    where
        T: Transport + Clone,
        P: Provider<T, Ethereum> + Send + Sync + Clone + 'static,
    {
        let message_queue: Address = "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"
            .parse()
            .map_err(|_| Error::WrongAddress)?;
        let replayer: Address = "0x5FC8d32690cc91D4c39d9d3abcBD16989F875707"
            .parse()
            .map_err(|_| Error::WrongAddress)?;
        let pk = hex!("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");
        let contracts =
            Contracts::new(provider, Some(pk), message_queue.0 .0, replayer.0 .0).unwrap();
        Ok(contracts)
    }

    struct DeploymentEnv {
        wvara_erc20: Address,
        verifier: Address,
        message_queue: Address,
        relayer: Address,
        erc20_treasury: Address,
        message_queue_proxy: Address,
        relayer_proxy: Address,
        erc20_treasury_proxy: Address,
    }

    async fn deploy<P, T>(provider: P) -> Result<DeploymentEnv, Error>
    where
        T: Transport + Clone,
        P: Provider<T, Ethereum> + Send + Sync + Clone + 'static,
    {
        let vwara_erc20_mock = ERC20Mock::deploy(provider.clone(), "wVARA".to_string())
            .await
            .map_err(Error::ErrorDuringContractExecution)?;
        let verifier_mock = VerifierMock::deploy(provider.clone())
            .await
            .map_err(Error::ErrorDuringContractExecution)?;

        let relayer = Relayer::deploy(provider.clone())
            .await
            .map_err(Error::ErrorDuringContractExecution)?;

        let erc20_treasury = ERC20Treasury::deploy(provider.clone())
            .await
            .map_err(Error::ErrorDuringContractExecution)?;

        let message_queue = MessageQueue::deploy(provider.clone())
            .await
            .map_err(Error::ErrorDuringContractExecution)?;

        let relayer_proxy =
            ProxyContract::deploy(provider.clone(), *relayer.address(), Bytes::new())
                .await
                .map_err(Error::ErrorDuringContractExecution)?;

        let message_queue_proxy =
            ProxyContract::deploy(provider.clone(), *message_queue.address(), Bytes::new())
                .await
                .map_err(Error::ErrorDuringContractExecution)?;

        let erc20_treasury_proxy =
            ProxyContract::deploy(provider.clone(), *message_queue.address(), Bytes::new())
                .await
                .map_err(Error::ErrorDuringContractExecution)?;

        Ok(DeploymentEnv {
            wvara_erc20: *vwara_erc20_mock.address(),
            verifier: *verifier_mock.address(),
            message_queue: *message_queue.address(),
            relayer: *relayer.address(),
            erc20_treasury: *erc20_treasury.address(),
            message_queue_proxy: *message_queue_proxy.address(),
            relayer_proxy: *relayer_proxy.address(),
            erc20_treasury_proxy: *erc20_treasury_proxy.address(),
        })
    }

    fn build_merkle_proof_json() -> String {
        let proof_json = r#"{
                "proof" : "203b6d7ee470fd6201aac1d849603241e3303f0ed38c6caeffeafa7708a700f0219f2065a8517c79e6c5dd7f3cf97709fea069f2e30787d283ea75461bcfb7231020f6d4cda614519936afcfd343abd4ec6620c722ca4ac82facdda42526927724e59115798dae55e08fbb386e18d9d843015168b94802845012f7943dd6e6560e90e844f40e7e20d1bbc1221f997cc57308601436354424e3ad38e5060dff630779a7b023f1af6923d9ec2d5f42ee311c387de28e24a5d4e689af858e8ff8b80182ca8d21874a644a26dafe33531d6f626aadd0436ff341ca72c5bad16506580c7e2ab7d32c38097c5ca47fe23bb118a75963b23ad671eff3edae03b30443ad28b05c94bb33b5dda0601a2e448e9bcff356a20aca2fca8548b3aa589d9ab3cf0661bc6e5fc4a2fd9cf752daa21d89c1c68300e0e6611d3461a6cf5b2111de14006cbc8af011601630a2940a972a880adfbe689f2bec6d53ecbda6a1408dece008702afebed1dbcf1be649d794abb58afac334310a248655ddba60e50076a05a206eaa36097d6572598071e178e79675c05ecf48bf64bb1fd19cb3df06c7c6af129bbdac42d8b090938ea97fc22f6cd607a44e168c625bf19254e1c4fe09b6a600b2f423299b72662a65ef56fce78a3ec88ade6ca54848619bf1da88764804b909d6f1e2d3e60e0b52622b64df9d56f5e743628b82c17a688be2b70cb37aef0211f854d5fa134e51a631225c700746d40ef9fdd8c10324949f4b50ab3ab25f5c1352fbaebb8b145be5c2f287899f0547d47254fd47a68ab2bdb4cfc6e9109d7a14d3b2e41225840451765085cd1799c88f270d6356e3a096cbf53a6f1c7838f5036e02246259487f2f340cd0d41ebe2b403e5596361f90c68fadde8aa891e7200b504aa7ff0b5dff127c695b0f7c33b4e1d4e57c03820ed492dc121796e096cc2ec27ee9037b56e0ca44693352ac335b687b757fdfb87136cfde7cf1865d54b9066ba8e5e9bdbf0fbdab7b1a02840ef1c415a51e74d9ef0812d9bd67e3a413b818d7fbab3649c5a5d8705d896f0a1a3b140d938486b99830c171108a862b0fa72e0943712e094e05cf1b5d50ee5422962bde5d533a4d7cc7ee7b2824148e71d81a3a3a8ec8091f8b52bc11ffe5189516441a01815250defe8d1e1e4150c4852c0ac274e45671a86b35be16b26f69bb60945f40e0caca8efbb998a268cf9db32927fd92d29a36c1b33d7bfe0540580c7a6628bcd28ead55135d8ad785b6e0424d1e870edf3353bad820bf5c7fa6e4fda335793fde58de57e062990001a8a30e07",
                "block_number" : 273,
                "merkle_root" : "0xa25559d02a45bf58afd5344964269d38e947a432c1097c342f937a4ad052a683"
            }"#;
        proof_json.to_string()
    }

    #[tokio::test]
    async fn test_deploy() -> Result<(), Error> {
        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .on_anvil_with_wallet();
        let deployment_env = deploy(provider).await?;

        assert_eq!(
            deployment_env.wvara_erc20,
            "0x5FbDB2315678afecb367f032d93F642f64180aa3"
                .parse::<Address>()
                .unwrap()
        );
        assert_eq!(
            deployment_env.verifier,
            "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"
                .parse::<Address>()
                .unwrap()
        );

        assert_eq!(
            deployment_env.relayer,
            "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"
                .parse::<Address>()
                .unwrap()
        );

        assert_eq!(
            deployment_env.erc20_treasury,
            "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"
                .parse::<Address>()
                .unwrap()
        );

        assert_eq!(
            deployment_env.message_queue,
            "0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9"
                .parse::<Address>()
                .unwrap()
        );

        assert_eq!(
            deployment_env.relayer_proxy,
            "0x5FC8d32690cc91D4c39d9d3abcBD16989F875707"
                .parse::<Address>()
                .unwrap()
        );

        assert_eq!(
            deployment_env.erc20_treasury_proxy,
            "0xa513E6E4b8f2a923D98304ec87F64353C4D5C853"
                .parse::<Address>()
                .unwrap()
        );

        assert_eq!(
            deployment_env.message_queue_proxy,
            "0x0165878A594ca255338adfa4d48449f69242Eb8F"
                .parse::<Address>()
                .unwrap()
        );

        Ok(())
    }

    #[tokio::test]
    async fn verify_block() {
        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .on_anvil_with_wallet();
        deploy(provider.clone()).await.unwrap();

        let contracts = build_contracts(provider).unwrap();

        let result = contracts
            .provide_merkle_root_json(build_merkle_proof_json().as_str())
            .await;

        assert!(result.is_ok());
    }

    #[test]
    fn verify_message_hash() {
        let msg = ContentMessage {
            sender: U256::from_be_bytes(H256::repeat_byte(3).to_fixed_bytes())
                .try_into()
                .unwrap(),
            receiver: Address::repeat_byte(3),
            nonce: B256::from(U256::from(3)),
            data: Bytes::from(vec![3, 3]),
        };

        let mut hash = msg.to_bytes();
        keccak_hash::keccak256(&mut hash);
        let hash = B256::from_slice(&hash[0..32]);

        let expected_hash: B256 = B256::from(
            U256::from_str_radix(
                "a366f34b585366d69a71c36c6831ec5d4588ff1fe04e8fb146865d86a9acead2",
                16,
            )
            .unwrap(),
        );

        assert_eq!(hash, expected_hash)
    }

    #[tokio::test]
    async fn verify_block_with_events() {
        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .on_anvil_with_wallet();
        let deployment_env = deploy(provider.clone()).await.unwrap();

        let contracts = build_contracts(provider).unwrap();

        let result = contracts
            .provide_merkle_root_json(build_merkle_proof_json().as_str())
            .await;

        assert!(result.is_ok());

        let logs = contracts.fetch_merkle_roots(10000).await.unwrap();

        assert_ne!(logs.len(), 0);

        for merkle_root_entry in logs.iter() {
            println!(
                "- Block : {} Merkle Root : {} TxHash : {}",
                merkle_root_entry.block_number,
                merkle_root_entry.merkle_root,
                merkle_root_entry.tx_hash
            )
        }
    }

    #[tokio::test]
    async fn verify_merkle_proof() {
        let hash0 = H256::random();
        let hash1 = H256::random();
        let hash2 = H256::random();

        let _leaf0 = Leaf::Hash(hash0);
        let _leaf1 = Leaf::Hash(hash1);
        let _leaf2 = Leaf::Hash(hash2);

        let leaves = vec![hash0, hash1, hash2];

        let _root = merkle_root::<KeccakHasher, _>(leaves.clone());
        let proof: MerkleProof<H256, H256> =
            merkle_proof::<KeccakHasher, Vec<H256>, H256>(leaves.clone(), 2);
        println!("leaves : {:?}", leaves);

        println!("Proof : {:?}", proof);
    }

    //TODO Check
    /*
    #[tokio::test]
    async fn verify_merkle_proof_for_msg() {
        let msg0 = ContentMessage {
            receiver: Address::repeat_byte(1),
            sender: U256::from_be_bytes(H256::repeat_byte(1).to_fixed_bytes())
                .try_into()
                .unwrap(),
            nonce: B256::from(U256::from(1)),
            data: Bytes::from(vec![1, 1]),
            //buf: Default::default(),
        };
        let msg1 = ContentMessage {
            receiver: Address::repeat_byte(2),
            sender: U256::from_be_bytes(H256::repeat_byte(2).to_fixed_bytes())
                .try_into()
                .unwrap(),
            nonce: B256::from(U256::from(2)),
            data: Bytes::from(vec![2, 2]),
            //buf: Default::default(),
        };
        let msg2 = ContentMessage {
            sender: U256::from_be_bytes(H256::repeat_byte(4).to_fixed_bytes())
                .try_into()
                .unwrap(),
            receiver: Address::repeat_byte(3),
            nonce: B256::from(U256::from(3)),
            data: Bytes::from(vec![3, 3]),
            //buf: Default::default(),
        };

        let leaves = vec![msg0.to_bytes(), msg1.to_bytes(), msg2.to_bytes()];

        let _root = merkle_root::<KeccakHasher, _>(leaves.clone());
        let proof: MerkleProof<H256, Vec<u8>> =
            merkle_proof::<KeccakHasher, _, Vec<u8>>(leaves.clone(), 2);

        let hash = keccak256(msg2.to_bytes());
        println!("Proof : {:?}", proof);
        println!("leaves : {:?}", leaves);
        println!("leaf hash: {:?}", hash);

        let is_ok = verify_proof::<KeccakHasher, _, _>(
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

        for i in 0..100u8 {
            let msg = ContentMessage {
                receiver: Address::repeat_byte(i),
                sender: U256::from_be_bytes(H256::repeat_byte(i).to_fixed_bytes())
                    .try_into()
                    .unwrap(),
                nonce: B256::left_padding_from(&vec![i]),
                data: Bytes::from(vec![i as u8, i as u8]),
            };
            leaves.push(msg.to_bytes())
        }

        let msg = ContentMessage {
            sender: U256::from_be_bytes(H256::repeat_byte(7).to_fixed_bytes())
                .try_into()
                .unwrap(),
            receiver: Address::parse_checksummed(
                "0xa513E6E4b8f2a923D98304ec87F64353C4D5C853",
                None,
            )
            .unwrap(),
            nonce: B256::from(U256::from(10)),
            data: Bytes::from(vec![3, 3, 3]),
        };
        leaves.push(msg.to_bytes());

        let _root = merkle_root::<KeccakHasher, _>(leaves.clone());
        let proof: MerkleProof<H256, Vec<u8>> =
            merkle_proof::<KeccakHasher, _, Vec<u8>>(leaves.clone(), 100);

        let hash = keccak256(msg.to_bytes());
        println!("Proof : {:?}", proof);
        println!("leaves : {:?}", leaves);
        println!("leaf hash: {:?}", hash);

        let is_ok = verify_proof::<KeccakHasher, _, _>(
            &proof.root,
            proof.proof,
            proof.number_of_leaves,
            proof.leaf_index,
            &proof.leaf,
        );

        println!("Ok: {:?}", is_ok);
    }
    */
}
