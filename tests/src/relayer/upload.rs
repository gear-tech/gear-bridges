//! Upload contracts required to run relayer onto the Gear Dev node.

use alloy::primitives::fixed_bytes;
use eth_events_electra_client::traits::EthEventsElectraFactory;
use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;

use crate::{connect_to_node, DEFAULT_BALANCE};
use gclient::GearApi;
use gstd::{ActorId, Encode};
use historical_proxy_client::traits::{HistoricalProxy, HistoricalProxyFactory};
use primitive_types::H160;
use relayer::message_relayer::eth_to_gear::api_provider::ApiProvider;
use sails_rs::{
    calls::{ActionIo, Activation, Call},
    gclient::calls::GClientRemoting,
};
use vft_client::traits::{VftAdmin, VftExtension, VftFactory};
use vft_manager_client::{
    traits::{VftManager, VftManagerFactory},
    Config, InitConfig, TokenSupply,
};

/// Contracts required to run Eth->Gear relayer.
#[allow(dead_code)]
pub(crate) struct EthContracts {
    pub api: GearApi,
    pub admin: ActorId,
    pub suri: String,

    pub mock_contract: ActorId,
    pub historical_proxy: ActorId,
    pub vft_manager: ActorId,
    pub eth_events: ActorId,
    /* vft contracts: there's exactly 3 different ERC20 tokens stored
       in test data. So we create 3 VFT tokens.
    */
    pub vft: [ActorId; 3],
}

impl EthContracts {
    pub async fn new() -> Self {
        let connection = connect_to_node(
            &[
                DEFAULT_BALANCE, /* mock-contract & admin */
                DEFAULT_BALANCE, /* historical-proxy */
                DEFAULT_BALANCE, /* vft-manager */
                DEFAULT_BALANCE, /* eth-events */
                DEFAULT_BALANCE, /* vft-1 */
                DEFAULT_BALANCE, /* vft-2 */
                DEFAULT_BALANCE, /* vft-3 */
            ],
            "eth->gear relayer",
            &[
                mock_contract::WASM_BINARY,
                historical_proxy::WASM_BINARY,
                vft_manager::WASM_BINARY,
                eth_events_electra::WASM_BINARY,
                vft::WASM_BINARY,
                vft::WASM_BINARY,
                vft::WASM_BINARY,
            ],
        )
        .await;

        let admin = connection.accounts[0].0;
        let suri = connection.accounts[0].2.clone();

        let salt = connection.salt;

        let api = connection
            .api
            .clone()
            .with(&suri)
            .expect("Failed to create API");
        let gas_limit = connection.gas_limit;

        let (message, mock_contract, _) = api
            .create_program_bytes(connection.code_ids[0], &salt, &[], gas_limit, 0)
            .await
            .expect("Failed to deploy mock contract");

        println!("mock-contract deployed at: {mock_contract:?}, message={message:?}");

        let remoting = GClientRemoting::new(api.clone());
        let mut vfts = [ActorId::zero(); 3];
        let vft_manager = {
            /* upload vft-manager and VFTs for each ERC20 token */

            let vft_manager = vft_manager_client::VftManagerFactory::new(remoting.clone())
                .new(InitConfig {
                    gear_bridge_builtin: Default::default(),
                    historical_proxy_address: Default::default(),
                    config: Config {
                        gas_for_token_ops: 10_000_000_000,
                        gas_for_reply_deposit: 10_000_000_000,
                        gas_to_send_request_to_builtin: 10_000_000_000,
                        gas_for_swap_token_maps: 1_500_000_000,
                        reply_timeout: 100,
                        fee_bridge: 0,
                        fee_incoming: 0,
                    },
                })
                .send_recv(connection.code_ids[2], connection.salt)
                .await
                .expect("Failed to deploy vft-manager");

            vft_manager_client::VftManager::new(remoting.clone())
                .unpause()
                .send_recv(vft_manager)
                .await
                .expect("Failed to unpause vft-manager");

            vft_manager_client::VftManager::new(remoting.clone())
                .update_erc_20_manager_address(H160(
                    fixed_bytes!("0xa84a9eac078195b32f914f845d7555c45c0ad936").0,
                ))
                .send_recv(vft_manager)
                .await
                .expect("Failed to update ERC20 manager address");

            let factory = vft_client::VftFactory::new(remoting.clone());

            let vft_accounts = &connection.accounts[4..7];
            let code_vft = &connection.code_ids[4..7];
            for (i, eth_token_id) in super::eth_to_gear::ETH_TOKEN_IDS.iter().enumerate() {
                println!(
                    "Creating VFT for ETH token ID: {:?}\nCode ID={:?}",
                    eth_token_id, code_vft[i]
                );

                let salt = vft_accounts[i].1.clone();
                let name = format!("TEST_TOKEN_{eth_token_id}");

                let vft_id = factory
                    .new(name, "TT".to_owned(), 20)
                    .send_recv(code_vft[i], salt)
                    .await
                    .expect("Failed to deploy VFT");

                vfts[i] = vft_id;

                let mut vft = vft_client::VftAdmin::new(remoting.clone());

                vft.set_minter(vft_manager)
                    .send_recv(vft_id)
                    .await
                    .expect("Failed to set VFT minter");

                vft.set_burner(vft_manager)
                    .send_recv(vft_id)
                    .await
                    .expect("Failed to set VFT burner");

                let mut vft_extension = vft_client::VftExtension::new(remoting.clone());

                while vft_extension
                    .allocate_next_balances_shard()
                    .send_recv(vft_id)
                    .await
                    .expect("Failed to allocate next balances shard")
                {}

                while vft_extension
                    .allocate_next_allowances_shard()
                    .send_recv(vft_id)
                    .await
                    .expect("Failed to allocate next allowances shard")
                {}

                let mut service = vft_manager_client::VftManager::new(remoting.clone());

                service
                    .map_vara_to_eth_address(vft_id, *eth_token_id, TokenSupply::Ethereum)
                    .send_recv(vft_manager)
                    .await
                    .expect("Failed to map VFT to ETH address");

                println!("Mapped VFT {vft_id:?} to ETH token ID: {eth_token_id:?}");
            }

            vft_manager
        };

        let eth_events = {
            eth_events_electra_client::EthEventsElectraFactory::new(remoting.clone())
                .new(mock_contract)
                .send_recv(connection.code_ids[3], connection.salt)
                .await
                .expect("Failed to deploy eth-events")
        };

        let historical_proxy = {
            /* upload historical proxy */
            let historical_proxy =
                historical_proxy_client::HistoricalProxyFactory::new(remoting.clone())
                    .new()
                    .send_recv(connection.code_ids[1], connection.salt)
                    .await
                    .expect("Failed to deploy historical proxy");

            let mut service = historical_proxy_client::HistoricalProxy::new(remoting.clone());

            let min_slot = super::eth_to_gear::TRANSACTIONS
                .iter()
                .map(|(_, tx_data)| tx_data.slot_number)
                .min()
                .expect("No transactions found");

            println!("Add endpoint #{min_slot}");
            service
                .add_endpoint(min_slot, eth_events)
                .send_recv(historical_proxy)
                .await
                .expect("Failed to add endpoint to historical proxy");

            vft_manager_client::VftManager::new(remoting.clone())
                .update_historical_proxy_address(historical_proxy)
                .send_recv(vft_manager)
                .await
                .expect("Failed to set historical proxy for VFT manager");

            historical_proxy
        };

        EthContracts {
            api,
            admin,
            suri,

            mock_contract,
            historical_proxy,
            vft_manager,
            eth_events,
            vft: vfts,
        }
    }
}

#[tokio::test]
async fn mock_contract_test() {
    let conn = connect_to_node(
        &[DEFAULT_BALANCE],
        "mock-contract",
        &[mock_contract::WASM_BINARY],
    )
    .await;

    let api = conn
        .api
        .with(&conn.accounts[0].2)
        .expect("Failed to create API");

    let (message, mock_contract, _) = api
        .create_program_bytes(conn.code_ids[0], &conn.salt, &[], conn.gas_limit, 0)
        .await
        .expect("Failed to deploy mock contract");

    println!("mock-contract deployed at: {mock_contract:?}, message={message:?}");

    let mut msg = Vec::with_capacity(
        checkpoint_light_client_client::service_checkpoint_for::io::Get::ROUTE.len() + 8,
    );

    msg.extend_from_slice(checkpoint_light_client_client::service_checkpoint_for::io::Get::ROUTE);
    4534630u64.encode_to(&mut msg);

    let msg = api
        .calculate_reply_for_handle(None, mock_contract, msg, conn.gas_limit, 0)
        .await
        .unwrap();

    assert!(msg
        .payload
        .starts_with(checkpoint_light_client_client::service_checkpoint_for::io::Get::ROUTE));

    let result: <checkpoint_light_client_client::service_checkpoint_for::io::Get as ActionIo>::Reply =
        gstd::Decode::decode(&mut &msg.payload[checkpoint_light_client_client::service_checkpoint_for::io::Get::ROUTE.len()..]).expect("Failed to decode reply");

    assert!(result.is_ok());
    let (checkpoint, block_root) = result.unwrap();

    assert_eq!(checkpoint, 4534655);
    assert_eq!(
        block_root.0,
        fixed_bytes!("0xca88b75653941bd709d239f9cf44aa0327d9532ce265db37f692de1df104a090").0
    );
}
