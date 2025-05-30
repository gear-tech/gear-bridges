use clap::{Args, Parser, Subcommand};
use gclient::{
    ext::sp_core::{Decode, Encode, H160, H256},
    EventProcessor, GearApi, WSAddress,
};
use gear_core::ids::ActorId;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: CliCommands,

    #[clap(flatten)]
    gear_client: GearClientArgs,
    #[clap(flatten)]
    contract: ContractArgs,
    #[clap(flatten)]
    builtin: BuiltinArg,
}

#[derive(Subcommand)]
enum CliCommands {
    /// Update contract implementations on etehreum network
    #[clap(visible_alias("update-impl"))]
    UpdateEthereumContract {
        /// New implementation address
        #[arg(long, env = "NEW_IMPLEMENTATION_ADDRESS")]
        new_implementation: String,
        /// Additional data passed to a new implementation after update
        #[arg(long, default_value = "0x")]
        call_data: String,
    },
    /// Change proxy admin on ethereum network
    #[clap(visible_alias("change-proxy-admin"))]
    ChangeProxyAdmin {
        /// New proxy admin address
        #[arg(long, env = "NEW_PROXY_ADMIN_ADDRESS")]
        new_admin: String,
    },
    /// Change governance address on etehreum network
    #[clap(visible_alias("change-gov"))]
    ChangeGovernanceAddress {
        /// New governance address
        #[arg(long, env = "NEW_GOVERNANCE_ADDRESS")]
        new_governance: String,
    },
}

#[derive(Args)]
struct GearClientArgs {
    /// Address of the Gear RPC endpoint
    #[arg(long, default_value = "wss://testnet.vara.network", env = "GEAR_RPC")]
    gear_endpoint: String,
    /// Port of the Gear RPC endpoint
    #[arg(long, default_value = "443", env = "GEAR_PORT")]
    gear_port: u16,
    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, env = "GEAR_SURI")]
    gear_suri: String,
}

#[derive(Args)]
struct ContractArgs {
    /// ProxyUpdater associated with the target contract
    #[arg(long, env = "CONTRACT_PROXY_UPDATER")]
    proxy_updater: String,
}

#[derive(Args)]
struct BuiltinArg {
    /// Address of the gear-eth-bridge builtin actor(defaults to the builtin address on testnet)
    #[arg(
        long,
        env = "BRIDGE_BUILTIN_ADDRESS",
        default_value = "0xf2816ced0b15749595392d3a18b5a2363d6fefe5b3b6153739f218151b7acdbf"
    )]
    bridge_builtin_address: String,
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    let cli = Cli::parse();

    let gear_api = create_gear_api(cli.gear_client).await;
    let bridge_builtin_actor = parse_builtin_actor_id(cli.builtin);
    let proxy_updater = decode_h160(&cli.contract.proxy_updater);

    let payload = match cli.command {
        CliCommands::UpdateEthereumContract {
            new_implementation,
            call_data,
        } => [
            &[0x00],
            decode_h160(&new_implementation).as_bytes(),
            &decode_hex(&call_data),
        ]
        .concat(),
        CliCommands::ChangeProxyAdmin { new_admin } => {
            [&[0x01], decode_h160(&new_admin).as_bytes()].concat()
        }
        CliCommands::ChangeGovernanceAddress { new_governance } => {
            [&[0x02], decode_h256(&new_governance).as_bytes()].concat()
        }
    };

    let request = gbuiltin_eth_bridge::Request::SendEthMessage {
        destination: proxy_updater,
        payload,
    };

    submit_builtin_request(gear_api, bridge_builtin_actor, request).await;
}

async fn create_gear_api(args: GearClientArgs) -> GearApi {
    GearApi::builder()
        .suri(args.gear_suri)
        .build(WSAddress::new(args.gear_endpoint, Some(args.gear_port)))
        .await
        .expect("Failed to create GearApi")
}

fn parse_builtin_actor_id(arg: BuiltinArg) -> ActorId {
    let data = decode_hex(&arg.bridge_builtin_address);
    ActorId::new(data.try_into().expect("Got input of wrong length"))
}

fn decode_h256(hex: &str) -> H256 {
    let data = decode_hex(hex);
    H256(data.try_into().expect("Wrond gear address length"))
}

fn decode_h160(hex: &str) -> H160 {
    let data = decode_hex(hex);
    H160(data.try_into().expect("Wrond ethereum address length"))
}

fn decode_hex(hex: &str) -> Vec<u8> {
    let formatted_hex = if &hex[..2] == "0x" { &hex[2..] } else { hex };
    hex::decode(formatted_hex).unwrap_or_else(|_| panic!("Failed to decode hex string {hex}"))
}

async fn submit_builtin_request(
    gear_api: GearApi,
    bridge_builtin_actor: ActorId,
    request: gbuiltin_eth_bridge::Request,
) {
    println!("Submitting request to the bridge built-in...");

    let payload = request.encode();

    let mut listener = gear_api
        .subscribe()
        .await
        .expect("Failed to crearte listener");

    // Use 95% of block gas limit for all extrinsics.
    let gas_limit = gear_api
        .block_gas_limit()
        .expect("Failed to get block gas limit")
        / 100
        * 95;

    let (message_id, block_hash) = gear_api
        .send_message_bytes(bridge_builtin_actor, payload, gas_limit, 0)
        .await
        .expect("Failed to send message to bridge builtin");

    let (_, reply, _value) = listener
        .reply_bytes_on(message_id)
        .await
        .unwrap_or_else(|_| panic!("Failed to get reply for message {message_id}"));
    let reply = reply.expect("Bridge builtin sent error reply");
    let reply = gbuiltin_eth_bridge::Response::decode(&mut &reply[..])
        .expect("Failed to decode bridge bultin reply");

    let block_number = gear_api
        .block_number_at(block_hash)
        .await
        .expect("Failed to query block hash by block number");

    match reply {
        gbuiltin_eth_bridge::Response::EthMessageQueued { nonce, .. } => {
            let mut nonce_le = [0; 32];
            nonce.to_little_endian(&mut nonce_le);

            println!("Message successfully submitted to bridge");
            println!("Nonce: {}({})", nonce, hex::encode(nonce_le));
            println!(
                "Block number: {} Block hash: {}",
                block_number,
                hex::encode(block_hash.0)
            );
        }
    };
}
