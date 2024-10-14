use clap::{Args, Parser, Subcommand};
use gclient::{EventListener, EventProcessor, GearApi, WSAddress};
use gear_core::ids::ProgramId;
use sails_rs::{calls::Call, gclient::calls::GClientRemoting, prelude::*};
use vft_client::{traits::Vft, Vft as VftClient};

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

    /// Whether to NOT revoke the admin role after the deployment
    #[arg(long, action)]
    dont_revoke_admin_role: bool,
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    let cli = Cli::parse();

    match cli.command {
        CliCommands::ExtendedVft(args) => {
            let address = WSAddress::new(&args.gear_endpoint, Some(args.gear_port));
            let gear_api = GearApi::init(address)
                .await
                .expect("Failed to initialize GearApi");
            let listener = gear_api
                .subscribe()
                .await
                .expect("Failed to subscribe to listener");

            let str_to_actorid = |s: String| {
                let s = if &s[..2] == "0x" { &s[2..] } else { &s };
                let data = hex::decode(s).expect("Failed to decode ActorId");
                ActorId::new(data.try_into().expect("Got input of wrong length"))
            };

            let params = ExtendedVftParams {
                name: args.token_name,
                symbol: args.token_symbol,
                decimals: args.token_decimals,

                mint_admin: args.mint_admin.map(str_to_actorid),
                burn_admin: args.burn_admin.map(str_to_actorid),

                revoke_admin_role: !args.dont_revoke_admin_role,
            };

            upload_extended_vft(params, gear_api, listener).await;
        }
    }
}

struct ExtendedVftParams {
    name: String,
    symbol: String,
    decimals: u8,
    mint_admin: Option<ActorId>,
    burn_admin: Option<ActorId>,
    revoke_admin_role: bool,
}

async fn upload_extended_vft(params: ExtendedVftParams, api: GearApi, mut listener: EventListener) {
    let payload = [
        "New".encode(),
        (params.name, params.symbol, params.decimals).encode(),
    ]
    .concat();

    let gas_limit = api
        .calculate_upload_gas(
            None,
            extended_vft_wasm::WASM_BINARY_OPT.to_vec(),
            payload.clone(),
            0,
            true,
        )
        .await
        .expect("Failed to calculate gas limit")
        .min_limit;

    let (message_id, program_id, _) = api
        .upload_program_bytes(
            extended_vft_wasm::WASM_BINARY_OPT,
            gclient::now_micros().to_le_bytes(),
            payload,
            gas_limit,
            0,
        )
        .await
        .expect("Failed to upload program");

    let program_id = ProgramId::from(program_id);

    assert!(listener
        .message_processed(message_id)
        .await
        .expect("Message is not processed")
        .succeed());

    let remoting = GClientRemoting::new(api.clone());
    let mut vft = VftClient::new(remoting);

    if let Some(minter) = params.mint_admin {
        vft.grant_minter_role(minter)
            .send_recv(program_id)
            .await
            .expect("Failed to grand minter role");

        println!("Granted minter role");
    }

    if let Some(burner) = params.burn_admin {
        vft.grant_burner_role(burner)
            .send_recv(program_id)
            .await
            .expect("Failed to grand burner role");

        println!("Granted burner role");
    }

    if params.revoke_admin_role {
        let current_admin = ActorId::new(*api.account_id().as_ref());

        vft.revoke_minter_role(current_admin)
            .send_recv(program_id)
            .await
            .expect("Failed to revoke minter role");

        vft.revoke_burner_role(current_admin)
            .send_recv(program_id)
            .await
            .expect("Failed to revoke burner role");

        vft.revoke_admin_role(current_admin)
            .send_recv(program_id)
            .await
            .expect("Failed to revoke admin role");

        println!("Revoked all admin roles from deployer");
    }

    println!("Program deployed at {:?}", program_id);
}
