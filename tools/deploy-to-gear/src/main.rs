use clap::{Args, Parser, Subcommand};
use gclient::{GearApi, WSAddress};
use gear_core::ids::prelude::*;
use sails_rs::{calls::*, gclient::calls::GClientRemoting, prelude::*};
use vft_client::traits::*;
use vft_vara_client::{traits::*, Mainnet};

const SIZE_MIGRATE_BATCH: u32 = 25;

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

    #[arg(long)]
    salt: Option<String>,

    #[command(subcommand)]
    command: CliCommands,
}

#[allow(clippy::enum_variant_names)]
#[derive(Subcommand)]
enum CliCommands {
    /// Deploy VFT contract
    Vft(VftArgs),
    /// Deploy VFT-VARA contract
    VftVara(RolesArgs),
    /// Deploy VFT contract for WUSDT
    Wusdt(RolesArgs),
    /// Deploy VFT contract for WUSDC
    Wusdc(RolesArgs),
    /// Deploy VFT contract for WETH
    Weth(RolesArgs),
    /// Deploy VFT contract for WBTC
    Wbtc(RolesArgs),
    AllocateShards {
        /// Program ID of the VFT contract
        program_id: String,
    },
    MigrateBalances(MigrateBalances),
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

    #[command(flatten)]
    roles: RolesArgs,
}

#[derive(Args)]
struct RolesArgs {
    /// ActorId that will be allowed to mint new tokens
    #[arg(long)]
    minter: Option<String>,
    /// ActorId that will be allowed to burn tokens
    #[arg(long)]
    burner: Option<String>,
}

#[derive(Args)]
struct MigrateBalances {
    #[arg(long, help = format!("Size of migration batch. Default: {SIZE_MIGRATE_BATCH}"))]
    size_batch: Option<u32>,
    /// ActorId of the source VFT contract (old)
    #[arg(long)]
    vft: String,
    /// ActorId of the destination VFT contract (new). Provided `remoting` should have account
    /// with mint-permission
    #[arg(long)]
    vft_new: String,
}

fn str_to_actorid(s: String) -> ActorId {
    let s = if &s[..2] == "0x" { &s[2..] } else { &s };
    let data = hex::decode(s).expect("Failed to decode ActorId");

    ActorId::new(data.try_into().expect("Got input of wrong length"))
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

    let salt = match cli.salt {
        Some(salt) => {
            let s = if &salt[..2] == "0x" {
                &salt[2..]
            } else {
                &salt
            };
            hex::decode(s)
                .inspect_err(|err| {
                    println!("Failed to decode salt: {err}, using random salt");
                })
                .ok()
        }
        _ => {
            println!("Salt is not provided, using random salt");
            None
        }
    };

    match cli.command {
        CliCommands::Vft(args) => {
            let minter = args.roles.minter.map(str_to_actorid);
            let burner = args.roles.burner.map(str_to_actorid);

            let uploader = Uploader::new(gear_api, minter, burner, salt);
            uploader
                .upload_vft(args.token_name, args.token_symbol, args.token_decimals)
                .await
        }

        CliCommands::VftVara(args) => {
            let minter = args.minter.map(str_to_actorid);
            let burner = args.burner.map(str_to_actorid);
            let uploader = Uploader::new(gear_api, minter, burner, salt);
            uploader.upload_vft_vara().await;
        }

        CliCommands::Wusdt(args) => {
            let minter = args.minter.map(str_to_actorid);
            let burner = args.burner.map(str_to_actorid);

            let uploader = Uploader::new(gear_api, minter, burner, salt);
            uploader
                .upload_vft("Bridged Tether USD".into(), "WUSDT".into(), 6)
                .await
        }

        CliCommands::Wusdc(args) => {
            let minter = args.minter.map(str_to_actorid);
            let burner = args.burner.map(str_to_actorid);

            let uploader = Uploader::new(gear_api, minter, burner, salt);
            uploader
                .upload_vft("Bridged USD Coin".into(), "WUSDC".into(), 6)
                .await
        }

        CliCommands::Weth(args) => {
            let minter = args.minter.map(str_to_actorid);
            let burner = args.burner.map(str_to_actorid);

            let uploader = Uploader::new(gear_api, minter, burner, salt);
            uploader
                .upload_vft("Bridged Wrapped Ether".into(), "WETH".into(), 18)
                .await
        }

        CliCommands::Wbtc(args) => {
            let minter = args.minter.map(str_to_actorid);
            let burner = args.burner.map(str_to_actorid);

            let uploader = Uploader::new(gear_api, minter, burner, salt);
            uploader
                .upload_vft("Bridged Wrapped BTC".into(), "WBTC".into(), 8)
                .await
        }

        CliCommands::AllocateShards { program_id } => {
            let program_id = str_to_actorid(program_id);
            let uploader = Uploader::new(gear_api, None, None, salt);
            uploader.allocate_shards(program_id).await;
        }

        CliCommands::MigrateBalances(args) => migrate_balances(gear_api, args).await,
    }
}

struct Uploader {
    api: GearApi,
    gas_limit: u64,
    minter: Option<ActorId>,
    burner: Option<ActorId>,
    salt: Option<Vec<u8>>,
}

impl Uploader {
    fn new(
        api: GearApi,
        minter: Option<ActorId>,
        burner: Option<ActorId>,
        salt: Option<Vec<u8>>,
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

    async fn allocate_shards(self, program_id: ActorId) {
        let remoting = GClientRemoting::new(self.api.clone());
        Self::allocate_shards_impl(remoting, program_id, self.gas_limit).await;
    }

    async fn allocate_shards_impl(remoting: GClientRemoting, program_id: ActorId, gas_limit: u64) {
        let mut vft_extension = vft_client::VftExtension::new(remoting);
        while vft_extension
            .allocate_next_balances_shard()
            .with_gas_limit(gas_limit)
            .send_recv(program_id)
            .await
            .expect("Failed to allocate next balances shard")
        {}

        while vft_extension
            .allocate_next_allowances_shard()
            .with_gas_limit(gas_limit)
            .send_recv(program_id)
            .await
            .expect("Failed to allocate next allowances shard")
        {}
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

        Self::allocate_shards_impl(remoting, program_id, self.gas_limit).await;
        println!("Program deployed");
    }

    async fn upload_vft(self, name: String, symbol: String, decimals: u8) {
        println!(
            r#"Upload VFT with: name = "{name}", symbol = "{symbol}", decimals = "{decimals}""#
        );

        let code_id = self.upload_code(vft::WASM_BINARY).await;
        println!("Code uploaded: {code_id:?}");

        let factory = vft_client::VftFactory::new(GClientRemoting::new(self.api.clone()));

        let salt = self
            .salt
            .clone()
            .unwrap_or_else(|| H256::random().0.to_vec());
        let program_id = factory
            .new(name, symbol, decimals)
            .with_gas_limit(self.gas_limit)
            .send_recv(code_id, &salt)
            .await
            .expect("Failed to upload program");

        self.upload_common(program_id).await
    }

    async fn upload_vft_vara(self) {
        let signer: gsdk::signer::Signer = self.api.clone().into();
        let network = if signer
            .api()
            .rpc()
            .system_chain()
            .await
            .expect("Determine chain name")
            == "Vara Network"
        {
            Mainnet::Yes
        } else {
            Mainnet::No
        };
        println!(
            "Deploy for the main network: {}",
            matches!(network, Mainnet::Yes)
        );

        let code_id = self.upload_code(vft_vara::WASM_BINARY).await;
        println!("Code uploaded: {code_id:?}");

        let factory = vft_vara_client::VftVaraFactory::new(GClientRemoting::new(self.api.clone()));

        let salt = self
            .salt
            .clone()
            .unwrap_or_else(|| H256::random().0.to_vec());

        let program_id = factory
            .new(network)
            .with_gas_limit(self.gas_limit)
            .send_recv(code_id, &salt)
            .await
            .expect("Failed to upload program");

        self.upload_common(program_id).await
    }
}

async fn migrate_balances(gear_api: GearApi, args: MigrateBalances) {
    let gas_limit = gear_api
        .block_gas_limit()
        .expect("Unable to get block gas limit");
    let size_batch = args.size_batch.unwrap_or(SIZE_MIGRATE_BATCH);

    if let Err(e) = gear_common::migrate_balances(
        GClientRemoting::new(gear_api),
        gas_limit,
        size_batch,
        str_to_actorid(args.vft),
        str_to_actorid(args.vft_new),
    )
    .await
    {
        println!("Failed to migrate balances: {e:?}");
    }
}
