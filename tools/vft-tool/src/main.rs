use bridging_payment_client::traits::BridgingPayment;
use clap::{Args, Parser, Subcommand, ValueEnum};
use cli_utils::GearConnectionArgs;
use gclient::GearApi;
use gear_common::api_provider::{ApiProvider, ApiProviderConnection};
use gear_core::ids::prelude::*;
use sails_rs::{calls::*, gclient::calls::GClientRemoting, prelude::*};
use vft_client::{traits::*, vft_admin::io};
use vft_manager_client::{traits::*, TokenSupply};
use vft_vara_client::{traits::*, Mainnet};

const SIZE_MIGRATE_BATCH: u32 = 200;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[clap(flatten)]
    gear_connection: GearConnectionArgs,

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
    HexEncodedMessage(HexEncodedMessageArgs),
    Manager {
        program_id: String,
        #[command(subcommand)]
        action: ManagerAction,
    },
    Token {
        program_id: String,
        #[command(subcommand)]
        action: TokenAction,
    },
    Payment {
        program_id: String,
        #[command(subcommand)]
        action: PaymentAction,
    },
    Roles {
        program_id: String,
        #[command(subcommand)]
        action: RolesAction,
    },
}

#[derive(Subcommand)]
enum ManagerAction {
    Status,
    SetErc20Manager {
        address: String,
    },
    Map {
        vft: String,
        erc20: String,
        supply: Supply,
    },
    RequestBridging {
        vft: String,
        amount: String,
        receiver: String,
    },
    RecoverInterruptedTransfer {
        message_id: String,
    },
    Unmap {
        vft: String,
    },
    Pause,
    Unpause,
}

#[derive(Clone, ValueEnum)]
enum Supply {
    Ethereum,
    Gear,
}
#[derive(Subcommand)]
enum TokenAction {
    Status {
        owner: String,
        spender: Option<String>,
    },
    Mint {
        to: String,
        amount: String,
    },
    Approve {
        spender: String,
        amount: String,
    },
    Wrap {
        amount: u128,
    },
    Unwrap {
        amount: String,
    },
}

#[derive(Subcommand)]
enum PaymentAction {
    Status,
    PayFees {
        nonce: String,
        #[arg(long)]
        value: Option<u128>,
    },
    PayPriorityFees {
        block: String,
        nonce: String,
        #[arg(long)]
        value: Option<u128>,
    },
}

#[derive(Subcommand)]
enum RolesAction {
    Status,
    SetMinter { actor: String },
    SetBurner { actor: String },
}

#[derive(Args)]
struct HexEncodedMessageArgs {
    #[command(subcommand)]
    message: HexEncodedMessage,
}

#[derive(Subcommand)]
enum HexEncodedMessage {
    Pause,
    Resume,
    Exit { vft_new: ActorId },
    SetBurner { burner: ActorId },
    SetMinter { minter: ActorId },
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

fn str_to_h160(s: String) -> H160 {
    let data = hex::decode(s.strip_prefix("0x").unwrap_or(&s)).expect("Invalid EVM address");
    H160::from_slice(&data)
}

fn print_encoded_message(message: HexEncodedMessage) {
    use HexEncodedMessage::*;

    let (label, encoded_call) = match message {
        Pause => ("Pause", io::Pause::encode_call()),
        Resume => ("Resume", io::Resume::encode_call()),

        Exit { vft_new } => ("Exit", io::Exit::encode_call(vft_new)),

        SetBurner { burner } => ("SetBurner", io::SetBurner::encode_call(burner)),

        SetMinter { minter } => ("SetMinter", io::SetMinter::encode_call(minter)),
    };

    println!(r#"{label}: "{}""#, hex::encode(encoded_call));
}

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    pretty_env_logger::formatted_timed_builder()
        .filter_level(log::LevelFilter::Info)
        .format_target(false)
        .format_timestamp_secs()
        .parse_default_env()
        .init();

    let cli = Cli::parse();
    if let CliCommands::HexEncodedMessage(args) = cli.command {
        print_encoded_message(args.message);

        return;
    }

    let endpoint = cli
        .gear_connection
        .get_endpoint()
        .expect("Invalid Gear URL");
    let api_provider = ApiProvider::new(endpoint, u8::MAX)
        .await
        .expect("Failed to initialize GearApi");
    let mut connection = api_provider.connection();

    api_provider.spawn();

    let gear_api = connection
        .gclient_client(&cli.gear_suri)
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

        CliCommands::MigrateBalances(args) => {
            migrate_balances(connection, cli.gear_suri, args).await
        }
        CliCommands::Manager { program_id, action } => {
            let program_id = str_to_actorid(program_id);
            let gas_limit = gear_api
                .block_gas_limit()
                .expect("Unable to get block gas limit");
            let mut manager = vft_manager_client::VftManager::new(GClientRemoting::new(gear_api));
            match action {
                ManagerAction::Status => {
                    println!(
                        "admin: {:?}",
                        manager
                            .admin()
                            .recv(program_id)
                            .await
                            .expect("Admin query failed")
                    );
                    println!(
                        "paused: {}",
                        manager
                            .is_paused()
                            .recv(program_id)
                            .await
                            .expect("Pause query failed")
                    );
                    println!(
                        "erc20_manager: {:?}",
                        manager
                            .erc_20_manager_address()
                            .recv(program_id)
                            .await
                            .expect("Manager address query failed")
                    );
                    println!(
                        "config: {:?}",
                        manager
                            .get_config()
                            .recv(program_id)
                            .await
                            .expect("Config query failed")
                    );
                    println!(
                        "mappings: {:?}",
                        manager
                            .vara_to_eth_addresses()
                            .recv(program_id)
                            .await
                            .expect("Mappings query failed")
                    );
                }
                ManagerAction::SetErc20Manager { address } => {
                    manager
                        .update_erc_20_manager_address(str_to_h160(address))
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Manager binding failed");
                    println!(
                        "erc20_manager: {:?}",
                        manager
                            .erc_20_manager_address()
                            .recv(program_id)
                            .await
                            .expect("Manager address query failed")
                    );
                }
                ManagerAction::Map { vft, erc20, supply } => {
                    let supply = match supply {
                        Supply::Ethereum => TokenSupply::Ethereum,
                        Supply::Gear => TokenSupply::Gear,
                    };
                    manager
                        .map_vara_to_eth_address(str_to_actorid(vft), str_to_h160(erc20), supply)
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Mapping failed");
                    println!(
                        "mappings: {:?}",
                        manager
                            .vara_to_eth_addresses()
                            .recv(program_id)
                            .await
                            .expect("Mappings query failed")
                    );
                }
                ManagerAction::RequestBridging {
                    vft,
                    amount,
                    receiver,
                } => {
                    let fee = manager
                        .get_config()
                        .recv(program_id)
                        .await
                        .expect("Config query failed")
                        .fee_incoming;
                    let result = manager
                        .request_bridging(
                            str_to_actorid(vft),
                            U256::from_dec_str(&amount).expect("Invalid amount"),
                            str_to_h160(receiver),
                        )
                        .with_value(fee)
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Bridge request failed");
                    println!("bridge: {:?}", result.expect("Bridge request rejected"));
                }
                ManagerAction::RecoverInterruptedTransfer { message_id } => {
                    let bytes: [u8; 32] = hex::decode(message_id.trim_start_matches("0x"))
                        .expect("Invalid message ID hex")
                        .try_into()
                        .expect("Message ID must be 32 bytes");
                    let result = manager
                        .handle_request_bridging_interrupted_transfer(MessageId::from(bytes))
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Interrupted-transfer recovery transport failed");
                    println!("recovery: {result:?}");
                }
                ManagerAction::Unmap { vft } => {
                    manager
                        .remove_vara_to_eth_address(str_to_actorid(vft))
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Unmapping failed");
                    println!(
                        "mappings: {:?}",
                        manager
                            .vara_to_eth_addresses()
                            .recv(program_id)
                            .await
                            .expect("Mappings query failed")
                    );
                }
                ManagerAction::Pause => {
                    manager
                        .pause()
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Pause failed");
                    println!(
                        "paused: {}",
                        manager
                            .is_paused()
                            .recv(program_id)
                            .await
                            .expect("Pause query failed")
                    );
                }
                ManagerAction::Unpause => {
                    manager
                        .unpause()
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Unpause failed");
                    println!(
                        "paused: {}",
                        manager
                            .is_paused()
                            .recv(program_id)
                            .await
                            .expect("Pause query failed")
                    );
                }
            }
        }
        CliCommands::Token { program_id, action } => {
            let program_id = str_to_actorid(program_id);
            let gas_limit = gear_api
                .block_gas_limit()
                .expect("Unable to get block gas limit");
            let remoting = GClientRemoting::new(gear_api);
            match action {
                TokenAction::Status { owner, spender } => {
                    let token = vft_client::Vft::new(remoting);
                    let owner = str_to_actorid(owner);
                    println!(
                        "balance: {:?}",
                        token
                            .balance_of(owner)
                            .recv(program_id)
                            .await
                            .expect("Balance query failed")
                    );
                    println!(
                        "total_supply: {:?}",
                        token
                            .total_supply()
                            .recv(program_id)
                            .await
                            .expect("Supply query failed")
                    );
                    if let Some(spender) = spender {
                        println!(
                            "allowance: {:?}",
                            token
                                .allowance(owner, str_to_actorid(spender))
                                .recv(program_id)
                                .await
                                .expect("Allowance query failed")
                        );
                    }
                }
                TokenAction::Mint { to, amount } => {
                    let mut admin = vft_client::VftAdmin::new(remoting);
                    admin
                        .mint(
                            str_to_actorid(to),
                            U256::from_dec_str(&amount).expect("Invalid amount"),
                        )
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Mint failed");
                    println!("minted");
                }
                TokenAction::Approve { spender, amount } => {
                    let mut token = vft_client::Vft::new(remoting);
                    let approved = token
                        .approve(
                            str_to_actorid(spender),
                            U256::from_dec_str(&amount).expect("Invalid amount"),
                        )
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Approve failed");
                    assert!(approved, "Approval rejected");
                    println!("approved");
                }
                TokenAction::Wrap { amount } => {
                    let mut native = vft_vara_client::VftNativeExchange::new(remoting);
                    native
                        .mint()
                        .with_value(amount)
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Native wrap failed");
                    println!("wrapped");
                }
                TokenAction::Unwrap { amount } => {
                    let mut native = vft_vara_client::VftNativeExchange::new(remoting);
                    let burned = native
                        .burn(U256::from_dec_str(&amount).expect("Invalid amount"))
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Native unwrap failed");
                    assert!(burned, "Native unwrap rejected");
                    println!("unwrapped");
                }
            }
        }
        CliCommands::Payment { program_id, action } => {
            let program_id = str_to_actorid(program_id);
            let gas_limit = gear_api
                .block_gas_limit()
                .expect("Unable to get block gas limit");
            let mut payment =
                bridging_payment_client::BridgingPayment::new(GClientRemoting::new(gear_api));
            let state = payment
                .get_state()
                .recv(program_id)
                .await
                .expect("Payment state query failed");
            match action {
                PaymentAction::Status => println!("payment: {state:?}"),
                PaymentAction::PayFees { nonce, value } => {
                    payment
                        .pay_fees(U256::from_dec_str(&nonce).expect("Invalid nonce"))
                        .with_value(value.unwrap_or(state.fee))
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Payment failed");
                    println!("paid");
                }
                PaymentAction::PayPriorityFees {
                    block,
                    nonce,
                    value,
                } => {
                    let block = hex::decode(block.strip_prefix("0x").unwrap_or(&block))
                        .expect("Invalid block hash");
                    let block = H256::from_slice(&block);
                    payment
                        .pay_priority_fees(
                            block,
                            U256::from_dec_str(&nonce).expect("Invalid nonce"),
                        )
                        .with_value(value.unwrap_or(state.priority_fee))
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Priority payment failed");
                    println!("priority paid");
                }
            }
        }
        CliCommands::Roles { program_id, action } => {
            let program_id = str_to_actorid(program_id);
            let gas_limit = gear_api
                .block_gas_limit()
                .expect("Unable to get block gas limit");
            let mut admin = vft_client::VftAdmin::new(GClientRemoting::new(gear_api));
            match action {
                RolesAction::Status => {
                    println!(
                        "minter: {:?}",
                        admin
                            .minter()
                            .recv(program_id)
                            .await
                            .expect("Minter query failed")
                    );
                    println!(
                        "burner: {:?}",
                        admin
                            .burner()
                            .recv(program_id)
                            .await
                            .expect("Burner query failed")
                    );
                }
                RolesAction::SetMinter { actor } => {
                    admin
                        .set_minter(str_to_actorid(actor))
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Set minter failed");
                    println!(
                        "minter: {:?}",
                        admin
                            .minter()
                            .recv(program_id)
                            .await
                            .expect("Minter query failed")
                    );
                }
                RolesAction::SetBurner { actor } => {
                    admin
                        .set_burner(str_to_actorid(actor))
                        .with_gas_limit(gas_limit)
                        .send_recv(program_id)
                        .await
                        .expect("Set burner failed");
                    println!(
                        "burner: {:?}",
                        admin
                            .burner()
                            .recv(program_id)
                            .await
                            .expect("Burner query failed")
                    );
                }
            }
        }

        CliCommands::HexEncodedMessage(..) => {}
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
        vft_client::allocate_shards(remoting, program_id, gas_limit)
            .await
            .expect("Failed to initialize VFT storage");
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
            io::SetMinter::ROUTE,
            vft_vara_client::vft_admin::io::SetMinter::ROUTE,
        );
        assert_eq!(
            io::SetBurner::ROUTE,
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
            .legacy()
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

async fn migrate_balances(
    connection: ApiProviderConnection,
    gear_suri: String,
    args: MigrateBalances,
) {
    let size_batch = args.size_batch.unwrap_or(SIZE_MIGRATE_BATCH);

    if let Err(e) = gear_common::migrate_balances(
        connection,
        gear_suri,
        size_batch,
        str_to_actorid(args.vft),
        str_to_actorid(args.vft_new),
    )
    .await
    {
        println!("Failed to migrate balances: {e:?}");
    }
}
