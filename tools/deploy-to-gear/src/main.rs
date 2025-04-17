use clap::{Args, Parser, Subcommand};
use gclient::{GearApi, WSAddress};
use gear_core::ids::prelude::*;
use sails_rs::{calls::*, gclient::calls::GClientRemoting, prelude::*};
use vft_client::traits::*;
use vft_vara_client::traits::*;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
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

    /// ActorId that will be allowed to mint new tokens
    #[arg(long = "mint-admin")]
    minter: Option<String>,
    /// ActorId that will be allowed to burn tokens
    #[arg(long = "burn-admin")]
    burner: Option<String>,

    #[arg(long = "salt")]
    salt: Option<H256>,

    #[command(subcommand)]
    command: CliCommands,
}

#[allow(clippy::enum_variant_names)]
#[derive(Subcommand)]
enum CliCommands {
    /// Deploy VFT contract
    Vft(VftArgs),
    /// Deploy VFT-VARA contract
    VftVara,
}

#[derive(Args)]
struct VftArgs {
    /// Name of the token that will be set during initialization
    #[arg(long = "token-name", short = 'n', default_value = "VftToken")]
    token_name: String,
    /// Symbol of the token that will be set during initialization
    #[arg(long = "token-symbol", short = 's', default_value = "VT")]
    token_symbol: String,
    /// Decimals of the token that will be set during initialization
    #[arg(long = "token-decimals", short = 'd', default_value = "18")]
    token_decimals: u8,
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    let cli = Cli::parse();

    let address = WSAddress::new(&cli.gear_endpoint, Some(cli.gear_port));
    let gear_api = GearApi::builder()
        .suri(cli.gear_suri)
        .build(address)
        .await
        .expect("Failed to initialize GearApi");

    let str_to_actorid = |s: String| {
        let s = if &s[..2] == "0x" { &s[2..] } else { &s };
        let data = hex::decode(s).expect("Failed to decode ActorId");

        ActorId::new(data.try_into().expect("Got input of wrong length"))
    };
    let minter = cli.minter.map(str_to_actorid);
    let burner = cli.burner.map(str_to_actorid);
    let updater = Uploader::new(gear_api, minter, burner, cli.salt);

    match cli.command {
        CliCommands::Vft(args) => {
            updater
                .upload_vft(args.token_name, args.token_symbol, args.token_decimals)
                .await
        }

        CliCommands::VftVara => updater.upload_vft_vara().await,
    }
}

struct Uploader {
    api: GearApi,
    gas_limit: u64,
    minter: Option<ActorId>,
    burner: Option<ActorId>,
    salt: Option<H256>,
}

impl Uploader {
    fn new(
        api: GearApi,
        minter: Option<ActorId>,
        burner: Option<ActorId>,
        salt: Option<H256>,
    ) -> Self {
        Self {
            gas_limit: api
                .block_gas_limit()
                .expect("Unable to get block gas limit"),
            api,
            minter,
            burner,
            salt,
        }
    }

    async fn upload_code(&self, wasm_binary: &[u8]) -> CodeId {
        self.api
            .upload_code(wasm_binary)
            .await
            .map(|(code_id, ..)| code_id)
            .unwrap_or_else(|_| CodeId::generate(wasm_binary))
    }

    async fn upload_common(self, program_id: ActorId) {
        assert_eq!(
            vft_client::vft_admin::io::SetMinter::ROUTE,
            vft_vara_client::vft_admin::io::SetMinter::ROUTE,
        );
        assert_eq!(
            vft_client::vft_admin::io::SetBurner::ROUTE,
            vft_vara_client::vft_admin::io::SetBurner::ROUTE,
        );
        assert_eq!(
            vft_client::vft_extension::io::AllocateNextAllowancesShard::ROUTE,
            vft_vara_client::vft_extension::io::AllocateNextAllowancesShard::ROUTE,
        );
        assert_eq!(
            vft_client::vft_extension::io::AllocateNextBalancesShard::ROUTE,
            vft_vara_client::vft_extension::io::AllocateNextBalancesShard::ROUTE,
        );

        println!("Program constructed: {program_id:?}");

        let remoting = GClientRemoting::new(self.api);
        let mut vft = vft_client::VftAdmin::new(remoting.clone());

        if let Some(minter) = self.minter {
            vft.set_minter(minter)
                .with_gas_limit(self.gas_limit)
                .send_recv(program_id)
                .await
                .expect("Failed to grand minter role");

            println!("Granted minter role");
        }

        if let Some(burner) = self.burner {
            vft.set_burner(burner)
                .with_gas_limit(self.gas_limit)
                .send_recv(program_id)
                .await
                .expect("Failed to grand burner role");

            println!("Granted burner role");
        }

        // Allocating underlying shards.
        let mut vft_extension = vft_client::VftExtension::new(remoting);
        while vft_extension
            .allocate_next_balances_shard()
            .with_gas_limit(self.gas_limit)
            .send_recv(program_id)
            .await
            .expect("Failed to allocate next balances shard")
        {}

        while vft_extension
            .allocate_next_allowances_shard()
            .with_gas_limit(self.gas_limit)
            .send_recv(program_id)
            .await
            .expect("Failed to allocate next allowances shard")
        {}

        println!("Program deployed");
    }

    async fn upload_vft(self, name: String, symbol: String, decimals: u8) {
        let code_id = self.upload_code(vft::WASM_BINARY).await;
        println!("Code uploaded: {code_id:?}");

        let factory = vft_client::VftFactory::new(GClientRemoting::new(self.api.clone()));

        let salt = self.salt.unwrap_or_else(H256::random);

        let program_id = factory
            .new(name, symbol, decimals)
            .with_gas_limit(self.gas_limit)
            .send_recv(code_id, salt.as_bytes())
            .await
            .expect("Failed to upload program");

        self.upload_common(program_id).await
    }

    async fn upload_vft_vara(self) {
        let code_id = self.upload_code(vft_vara::WASM_BINARY).await;
        println!("Code uploaded: {code_id:?}");

        let factory = vft_vara_client::VftVaraFactory::new(GClientRemoting::new(self.api.clone()));

        let salt = self.salt.unwrap_or_else(H256::random);

        let program_id = factory
            .new()
            .with_gas_limit(self.gas_limit)
            .send_recv(code_id, salt.as_bytes())
            .await
            .expect("Failed to upload program");

        self.upload_common(program_id).await
    }
}
