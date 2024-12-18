use alloy::{
    hex::FromHex,
    network::{Ethereum, EthereumWallet, NetworkWallet},
    primitives::{Address, Bytes, B256, U256},
    providers::{Provider, ProviderBuilder, WalletProvider},
    sol,
    transports::Transport,
};

use binary_merkle_tree::{merkle_proof, merkle_root, Leaf, MerkleProof};
use primitive_types::H256;
use sp_core::KeccakHasher;

use crate::{abi::ContentMessage, Contracts, Error};

use serde::{Deserialize, Serialize};

sol!(
    #[sol(rpc)]
    ProxyContract,
    "../out/ProxyContract.sol/ProxyContract.json"
);

sol!(
    #[sol(rpc)]
    ERC20Mock,
    "../out/ERC20Mock.sol/ERC20Mock.json"
);

sol!(
    #[sol(rpc)]
    Relayer,
    "../out/Relayer.sol/Relayer.json"
);

sol!(
    #[sol(rpc)]
    MessageQueue,
    "../out/MessageQueue.sol/MessageQueue.json"
);

sol!(
    #[sol(rpc)]
    Verifier,
    "../out/Verifier.sol/Verifier.json"
);

sol!(
    #[sol(rpc)]
    VerifierMock,
    "../out/VerifierMock.sol/Verifier.json"
);

sol!(
    #[sol(rpc)]
    ERC20Manager,
    "../out/ERC20Manager.sol/ERC20Manager.json"
);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockMerkleRootProof {
    pub proof: Bytes,
    pub block_number: U256,
    pub merkle_root: B256,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct DeploymentEnv {
    pub wvara_erc20: Address,
    pub verifier: Address,
    pub message_queue: Address,
    pub relayer: Address,
    pub erc20_manager: Address,
    pub message_queue_proxy: Address,
    pub relayer_proxy: Address,
    pub erc20_manager_proxy: Address,
}

fn build_contracts<P, T>(
    provider: P,
    env: &DeploymentEnv,
) -> Result<Contracts<P, T, Ethereum>, Error>
where
    T: Transport + Clone,
    P: Provider<T, Ethereum> + Send + Sync + Clone + 'static,
{
    let contracts =
        Contracts::new(provider, *env.message_queue_proxy.0, *env.relayer_proxy.0).unwrap();
    Ok(contracts)
}

async fn deploy<P, T>(provider: P, signer_address: Address) -> Result<DeploymentEnv, Error>
where
    T: Transport + Clone,
    P: Provider<T, Ethereum> + Send + Sync + Clone + 'static,
{
    let relayer_proxy = ProxyContract::deploy(provider.clone())
        .await
        .map_err(Error::ErrorDuringContractExecution)?;

    let message_queue_proxy = ProxyContract::deploy(provider.clone())
        .await
        .map_err(Error::ErrorDuringContractExecution)?;

    let erc20_manager_proxy = ProxyContract::deploy(provider.clone())
        .await
        .map_err(Error::ErrorDuringContractExecution)?;

    let vwara_erc20_mock = ERC20Mock::deploy(provider.clone(), "wVARA".to_string())
        .await
        .map_err(Error::ErrorDuringContractExecution)?;
    let verifier_mock = VerifierMock::deploy(provider.clone())
        .await
        .map_err(Error::ErrorDuringContractExecution)?;

    let relayer = Relayer::deploy(provider.clone(), *verifier_mock.address())
        .await
        .map_err(Error::ErrorDuringContractExecution)?;

    let erc20_manager = ERC20Manager::deploy(
        provider.clone(),
        *message_queue_proxy.address(),
        Default::default(),
    )
    .await
    .map_err(Error::ErrorDuringContractExecution)?;

    let message_queue = MessageQueue::deploy(provider.clone(), *relayer_proxy.address())
        .await
        .map_err(Error::ErrorDuringContractExecution)?;

    let deployment = DeploymentEnv {
        wvara_erc20: *vwara_erc20_mock.address(),
        verifier: *verifier_mock.address(),
        message_queue: *message_queue.address(),
        relayer: *relayer.address(),
        erc20_manager: *erc20_manager.address(),
        message_queue_proxy: *message_queue_proxy.address(),
        relayer_proxy: *relayer_proxy.address(),
        erc20_manager_proxy: *erc20_manager_proxy.address(),
    };

    let nonce = provider.get_transaction_count(signer_address).await?;

    let relayer_proxy = ProxyContract::new(deployment.relayer_proxy, provider.clone());

    let pending_tx = relayer_proxy
        .upgradeToAndCall(deployment.relayer, Bytes::new())
        .nonce(nonce);

    let pending_tx = pending_tx
        .send()
        .await
        .map_err(Error::ErrorDuringContractExecution)?;

    pending_tx
        .get_receipt()
        .await
        .map_err(|_| Error::ErrorWaitingTransactionReceipt)?;

    let nonce = provider.get_transaction_count(signer_address).await?;

    let pending_tx = message_queue_proxy
        .upgradeToAndCall(deployment.message_queue, Bytes::new())
        .nonce(nonce);

    let pending_tx = pending_tx
        .send()
        .await
        .map_err(Error::ErrorDuringContractExecution)?;

    pending_tx
        .get_receipt()
        .await
        .map_err(|_| Error::ErrorWaitingTransactionReceipt)?;

    let nonce = provider.get_transaction_count(signer_address).await?;

    let pending_tx = erc20_manager_proxy
        .upgradeToAndCall(deployment.erc20_manager, Bytes::new())
        .nonce(nonce);

    let pending_tx = pending_tx
        .send()
        .await
        .map_err(Error::ErrorDuringContractExecution)?;

    pending_tx
        .get_receipt()
        .await
        .map_err(|_| Error::ErrorWaitingTransactionReceipt)?;

    Ok(deployment.clone())
}

fn get_test_block_merkle_proof() -> BlockMerkleRootProof {
    let merkle_root =
        B256::from_hex("0xa25559d02a45bf58afd5344964269d38e947a432c1097c342f937a4ad052a683")
            .unwrap();
    let proof  = Bytes::from_hex("203b6d7ee470fd6201aac1d849603241e3303f0ed38c6caeffeafa7708a700f0219f2065a8517c79e6c5dd7f3cf97709fea069f2e30787d283ea75461bcfb7231020f6d4cda614519936afcfd343abd4ec6620c722ca4ac82facdda42526927724e59115798dae55e08fbb386e18d9d843015168b94802845012f7943dd6e6560e90e844f40e7e20d1bbc1221f997cc57308601436354424e3ad38e5060dff630779a7b023f1af6923d9ec2d5f42ee311c387de28e24a5d4e689af858e8ff8b80182ca8d21874a644a26dafe33531d6f626aadd0436ff341ca72c5bad16506580c7e2ab7d32c38097c5ca47fe23bb118a75963b23ad671eff3edae03b30443ad28b05c94bb33b5dda0601a2e448e9bcff356a20aca2fca8548b3aa589d9ab3cf0661bc6e5fc4a2fd9cf752daa21d89c1c68300e0e6611d3461a6cf5b2111de14006cbc8af011601630a2940a972a880adfbe689f2bec6d53ecbda6a1408dece008702afebed1dbcf1be649d794abb58afac334310a248655ddba60e50076a05a206eaa36097d6572598071e178e79675c05ecf48bf64bb1fd19cb3df06c7c6af129bbdac42d8b090938ea97fc22f6cd607a44e168c625bf19254e1c4fe09b6a600b2f423299b72662a65ef56fce78a3ec88ade6ca54848619bf1da88764804b909d6f1e2d3e60e0b52622b64df9d56f5e743628b82c17a688be2b70cb37aef0211f854d5fa134e51a631225c700746d40ef9fdd8c10324949f4b50ab3ab25f5c1352fbaebb8b145be5c2f287899f0547d47254fd47a68ab2bdb4cfc6e9109d7a14d3b2e41225840451765085cd1799c88f270d6356e3a096cbf53a6f1c7838f5036e02246259487f2f340cd0d41ebe2b403e5596361f90c68fadde8aa891e7200b504aa7ff0b5dff127c695b0f7c33b4e1d4e57c03820ed492dc121796e096cc2ec27ee9037b56e0ca44693352ac335b687b757fdfb87136cfde7cf1865d54b9066ba8e5e9bdbf0fbdab7b1a02840ef1c415a51e74d9ef0812d9bd67e3a413b818d7fbab3649c5a5d8705d896f0a1a3b140d938486b99830c171108a862b0fa72e0943712e094e05cf1b5d50ee5422962bde5d533a4d7cc7ee7b2824148e71d81a3a3a8ec8091f8b52bc11ffe5189516441a01815250defe8d1e1e4150c4852c0ac274e45671a86b35be16b26f69bb60945f40e0caca8efbb998a268cf9db32927fd92d29a36c1b33d7bfe0540580c7a6628bcd28ead55135d8ad785b6e0424d1e870edf3353bad820bf5c7fa6e4fda335793fde58de57e062990001a8a30e07").unwrap();
    let block_number = U256::from(273);
    BlockMerkleRootProof {
        proof,
        block_number,
        merkle_root,
    }
}

#[tokio::test]
async fn verify_block() {
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .on_anvil_with_wallet();

    let signer_address = provider.wallet();
    let signer_address =
        <EthereumWallet as NetworkWallet<Ethereum>>::default_signer_address(signer_address);

    let deployment_env = deploy(provider.clone(), signer_address).await.unwrap();

    let contracts = build_contracts(provider, &deployment_env).unwrap();

    let block_proof = get_test_block_merkle_proof();

    let result = contracts
        .provide_merkle_root(
            block_proof.block_number,
            block_proof.merkle_root,
            block_proof.proof,
        )
        .await;

    assert!(result.is_ok());
}

#[test]
fn verify_message_hash() {
    let msg = ContentMessage {
        sender: U256::from_be_bytes(H256::repeat_byte(3).to_fixed_bytes()).into(),
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
#[ignore = "Fails for now"]
async fn verify_block_with_events() {
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .on_anvil_with_wallet();

    let signer_address = provider.wallet();
    let signer_address =
        <EthereumWallet as NetworkWallet<Ethereum>>::default_signer_address(signer_address);

    let deployment_env = deploy(provider.clone(), signer_address).await.unwrap();

    let contracts = build_contracts(provider, &deployment_env).unwrap();

    let block_proof = get_test_block_merkle_proof();

    let result = contracts
        .provide_merkle_root(
            block_proof.block_number,
            block_proof.merkle_root,
            block_proof.proof,
        )
        .await;

    assert!(result.is_ok());

    let logs = contracts.fetch_merkle_roots(10000).await.unwrap();

    assert_ne!(logs.len(), 0);
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
    assert_eq!(proof.number_of_leaves, 3);
}
