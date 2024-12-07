use clap::{Args, Parser, Subcommand};
use gclient::{
    ext::sp_core::{Decode, Encode, H160},
    EventProcessor, GearApi, WSAddress,
};
use gear_core::ids::ActorId;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: CliCommands,
}

#[derive(Subcommand)]
enum CliCommands {
    /// Update contracts on etehreum network
    #[clap(visible_alias("update-eth"))]
    UpdateEthereumContract {
        #[clap(flatten)]
        gear_client: GearClientArgs,
        #[clap(flatten)]
        contract: ContractArgs,
        #[clap(flatten)]
        builtin: BuiltinArg,
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
    /// Address of the new contract implementation
    #[arg(long, env = "NEW_IMPLEMENTATION_ADDRESS")]
    new_implementation: String,
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

    match cli.command {
        CliCommands::UpdateEthereumContract {
            gear_client,
            contract,
            builtin,
        } => {
            let gear_api = create_gear_api(gear_client).await;
            let bridge_builtin_actor = parse_builtin_actor_id(builtin);
            let contract_info = ContractInfo::parse(contract);

            update_ethereum_contract(gear_api, bridge_builtin_actor, contract_info).await;
        }
    }
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

struct ContractInfo {
    proxy_updater: H160,
    new_implementation: H160,
}

impl ContractInfo {
    fn parse(args: ContractArgs) -> Self {
        let proxy_updater = decode_hex(&args.proxy_updater);
        let proxy_updater = H160(
            proxy_updater
                .try_into()
                .expect("Wrond proxy_updater ethereum address length"),
        );

        let new_implementation = decode_hex(&args.new_implementation);
        let new_implementation = H160(
            new_implementation
                .try_into()
                .expect("Wrond new_implementation ethereum address length"),
        );

        Self {
            proxy_updater,
            new_implementation,
        }
    }
}

fn decode_hex(hex: &str) -> Vec<u8> {
    let formatted_hex = if &hex[..2] == "0x" { &hex[2..] } else { hex };
    hex::decode(formatted_hex).unwrap_or_else(|_| panic!("Failed to decode hex string {}", hex))
}

async fn update_ethereum_contract(
    gear_api: GearApi,
    bridge_builtin_actor: ActorId,
    contract_info: ContractInfo,
) {
    let discriminator = 0x00;
    let mut payload = vec![discriminator];
    payload.append(&mut contract_info.new_implementation.as_bytes().to_vec());

    let payload = gbuiltin_eth_bridge::Request::SendEthMessage {
        destination: contract_info.proxy_updater,
        payload,
    }
    .encode();

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

    let (message_id, _) = gear_api
        .send_message_bytes(bridge_builtin_actor, payload, gas_limit, 0)
        .await
        .expect("Failed to send message to bridge builtin");

    let (_, reply, _value) = listener
        .reply_bytes_on(message_id)
        .await
        .unwrap_or_else(|_| panic!("Failed to get reply for message {}", message_id));
    let reply = reply.expect("Bridge builtin sent error reply");
    let reply = gbuiltin_eth_bridge::Response::decode(&mut &reply[..])
        .expect("Failed to decode bridge bultin reply");

    let _nonce = match reply {
        gbuiltin_eth_bridge::Response::EthMessageQueued { nonce, .. } => {
            println!("Message successfully submitted to bridge. Nonce: {}", nonce);
            nonce
        }
    };
}
