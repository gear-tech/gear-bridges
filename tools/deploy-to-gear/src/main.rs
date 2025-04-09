use clap::{Args, Parser, Subcommand};
use gclient::{GearApi, WSAddress};
use gear_core::ids::prelude::*;
use sails_rs::{calls::*, gclient::calls::GClientRemoting, prelude::*};
use vft_client::traits::*;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: CliCommands,
}

#[allow(clippy::enum_variant_names)]
#[derive(Subcommand)]
enum CliCommands {
    /// Deploy extended vft contract
    #[clap(visible_alias("ev"))]
    ExtendedVft(ExtendedVftArgs),
}

#[derive(Args)]
struct ExtendedVftArgs {
    /// Address of the Gear RPC endpoint
    #[arg(
        long = "gear-endpoint",
        default_value = "wss://testnet.vara.network",
        env = "GEAR_RPC"
    )]
    gear_endpoint: String,

    /// Port of the Gear RPC endpoint
    #[arg(long = "gear-port", default_value = "443", env = "GEAR_PORT")]
    gear_port: u16,

    /// Substrate URI that identifies a user by a mnemonic phrase or
    /// provides default users from the keyring (e.g., "//Alice", "//Bob",
    /// etc.). The password for URI should be specified in the same `suri`,
    /// separated by the ':' char
    #[arg(long, default_value = "//Alice", env = "GEAR_SURI")]
    gear_suri: String,

    /// Name of the token that will be set during initialization
    #[arg(long = "token-name", short = 'n', default_value = "VftToken")]
    token_name: String,
    /// Symbol of the token that will be set during initialization
    #[arg(long = "token-symbol", short = 's', default_value = "VT")]
    token_symbol: String,
    /// Decimals of the token that will be set during initialization
    #[arg(long = "token-decimals", short = 'd', default_value = "18")]
    token_decimals: u8,

    /// ActorId that will be allowed to mint new tokens
    #[arg(long = "mint-admin")]
    mint_admin: Option<String>,
    /// ActorId that will be allowed to burn tokens
    #[arg(long = "burn-admin")]
    burn_admin: Option<String>,
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    let cli = Cli::parse();

    match cli.command {
        CliCommands::ExtendedVft(args) => {
            let address = WSAddress::new(&args.gear_endpoint, Some(args.gear_port));
            let gear_api = GearApi::builder()
                .suri(args.gear_suri)
                .build(address)
                .await
                .expect("Failed to initialize GearApi");

            let str_to_actorid = |s: String| {
                let s = if &s[..2] == "0x" { &s[2..] } else { &s };
                let data = hex::decode(s).expect("Failed to decode ActorId");
                ActorId::new(data.try_into().expect("Got input of wrong length"))
            };

            let params = VftParams {
                name: args.token_name,
                symbol: args.token_symbol,
                decimals: args.token_decimals,

                mint_admin: args.mint_admin.map(str_to_actorid),
                burn_admin: args.burn_admin.map(str_to_actorid),
            };

            upload_extended_vft(params, gear_api).await;
        }
    }
}

struct VftParams {
    name: String,
    symbol: String,
    decimals: u8,
    mint_admin: Option<ActorId>,
    burn_admin: Option<ActorId>,
}

async fn upload_extended_vft(params: VftParams, api: GearApi) {
    let gas_limit = api
        .block_gas_limit()
        .expect("Unable to get block gas limit");
    let code_id = api
        .upload_code(vft::WASM_BINARY)
        .await
        .map(|(code_id, ..)| code_id)
        .unwrap_or_else(|_| CodeId::generate(vft::WASM_BINARY));
    println!("Code uploaded: {code_id:?}");

    let remoting = GClientRemoting::new(api);
    let factory = vft_client::VftFactory::new(remoting.clone());

    let program_id = factory
        .new(params.name, params.symbol, params.decimals)
        .with_gas_limit(gas_limit)
        .send_recv(code_id, [])
        .await
        .expect("Failed to upload program");
    println!("Program constructed");

    let mut vft = vft_client::VftAdmin::new(remoting.clone());

    if let Some(minter) = params.mint_admin {
        vft.set_minter(minter)
            .send_recv(program_id)
            .await
            .expect("Failed to grand minter role");

        println!("Granted minter role");
    }

    if let Some(burner) = params.burn_admin {
        vft.set_burner(burner)
            .send_recv(program_id)
            .await
            .expect("Failed to grand burner role");

        println!("Granted burner role");
    }

    // Allocating underlying shards.
    let mut vft_extension = vft_client::VftExtension::new(remoting);
    while vft_extension
        .allocate_next_balances_shard()
        .send_recv(program_id)
        .await
        .expect("Failed to allocate next balances shard")
    {}

    while vft_extension
        .allocate_next_allowances_shard()
        .send_recv(program_id)
        .await
        .expect("Failed to allocate next allowances shard")
    {}

    println!("Program deployed at {program_id:?}");
}
