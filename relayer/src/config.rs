use crate::{
    cli::{self, GearEthCoreArgs, DEFAULT_COUNT_CONFIRMATIONS, DEFAULT_COUNT_THREADS},
    merkle_roots::{CriticalThreshold, MerkleRootRelayerOptions, SpikeConfig, StartupSyncStrategy},
};
use anyhow::{anyhow, Context};
use primitive_types::H256;
use prover::{consts::BLAKE2_DIGEST_SIZE, proving::GenesisConfig};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

const DEFAULT_PROMETHEUS_ENDPOINT: &str = "0.0.0.0:9090";
const DEFAULT_RELAYER_ID: &str = "default";
const DEFAULT_FILE_SYSTEM_PROOF_STORAGE: &str = "./proof_storage";
const DEFAULT_GEAR_PROOF_STORAGE_CONFIG: &str = "./onchain_proof_storage_data";
const DEFAULT_GNARK_DATA_PATH: &str = "data";

#[derive(Clone)]
pub struct EffectiveConfig {
    pub prometheus_endpoint: String,
    pub relayers: Vec<EffectiveRelayerConfig>,
}

#[derive(Clone)]
pub struct EffectiveRelayerConfig {
    pub id: String,
    pub priority: i64,
    pub gear: EffectiveGearConfig,
    pub ethereum: EffectiveEthereumConfig,
    pub http: EffectiveHttpConfig,
    pub storage: EffectiveStorageConfig,
    pub proof_storage: EffectiveProofStorageConfig,
    pub options: MerkleRootRelayerOptions,
}

#[derive(Clone)]
pub struct EffectiveGearConfig {
    pub endpoint: String,
    pub max_reconnect_attempts: u8,
}

#[derive(Clone)]
pub struct EffectiveEthereumConfig {
    pub endpoint: String,
    pub message_queue_address: String,
    pub fee_payer: String,
    pub max_retries: Option<u32>,
    pub retry_interval_ms: Option<u64>,
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
}

#[derive(Clone)]
pub struct EffectiveHttpConfig {
    pub address: String,
    pub token: String,
}

#[derive(Clone)]
pub struct EffectiveStorageConfig {
    pub block_storage: PathBuf,
}

#[derive(Clone)]
pub enum EffectiveProofStorageConfig {
    FileSystem {
        path: PathBuf,
    },
    Gear {
        fee_payer: String,
        config_dir: PathBuf,
    },
}

impl EffectiveConfig {
    pub fn from_cli(args: &GearEthCoreArgs) -> anyhow::Result<Self> {
        let ethereum_endpoint = required(
            args.ethereum_args.ethereum_endpoint.as_deref(),
            "--ethereum-endpoint/ETH_RPC",
        )?;
        let message_queue_address = required(
            args.ethereum_args.mq_address.as_deref(),
            "--mq-address/ETH_MESSAGE_QUEUE_ADDRESS",
        )?;
        let fee_payer = required(
            args.ethereum_args.eth_fee_payer.as_deref(),
            "--eth-fee-payer/ETH_FEE_PAYER",
        )?;
        let authority_set_hash = required(
            args.genesis_config_args.authority_set_hash.as_deref(),
            "--authority-set-hash/GENESIS_CONFIG_AUTHORITY_SET_HASH",
        )?;
        let authority_set_id = args
            .genesis_config_args
            .authority_set_id
            .ok_or_else(|| anyhow!("Missing --authority-set-id/GENESIS_CONFIG_AUTHORITY_SET_ID"))?;
        let web_server_token = required(
            args.web_server_token.as_deref(),
            "--web-server-token/WEB_SERVER_TOKEN",
        )?;
        let block_storage = args
            .block_storage_args
            .path
            .clone()
            .ok_or_else(|| anyhow!("No block storage path provided"))?;

        let proof_storage = match args.proof_storage_args.gear_fee_payer.clone() {
            Some(fee_payer) => EffectiveProofStorageConfig::Gear {
                fee_payer,
                config_dir: DEFAULT_GEAR_PROOF_STORAGE_CONFIG.into(),
            },
            None => EffectiveProofStorageConfig::FileSystem {
                path: DEFAULT_FILE_SYSTEM_PROOF_STORAGE.into(),
            },
        };

        let options = build_options(OptionSource {
            relayer_id: DEFAULT_RELAYER_ID,
            priority: 0,
            confirmations_merkle_root: args.confirmations_merkle_root,
            start_authority_set_id: args.start_authority_set_id,
            bridging_payment_address: args.bridging_payment_address.as_deref(),
            thread_count: match args.thread_count {
                Some(thread_count) => thread_count.into(),
                None => Some(DEFAULT_COUNT_THREADS),
            },
            critical_threshold: args.critical_threshold.clone(),
            startup_sync_strategy: args.startup_sync_strategy,
            startup_sync_blocks: args.startup_sync_blocks.clone(),
            spike_window: args.spike_window,
            spike_timeout: args.spike_timeout,
            priority_spike_timeout: args.priority_spike_timeout,
            spike_threshold: args.spike_threshold,
            save_interval: args.save_interval,
            check_interval: args.check_interval,
            gnark_data_path: PathBuf::from(DEFAULT_GNARK_DATA_PATH),
            authority_set_hash,
            authority_set_id,
        })?;

        let relayer = EffectiveRelayerConfig {
            id: DEFAULT_RELAYER_ID.to_string(),
            priority: 0,
            gear: EffectiveGearConfig {
                endpoint: args.gear_args.get_endpoint()?,
                max_reconnect_attempts: args.gear_args.max_reconnect_attempts,
            },
            ethereum: EffectiveEthereumConfig {
                endpoint: ethereum_endpoint.to_string(),
                message_queue_address: message_queue_address.to_string(),
                fee_payer: fee_payer.to_string(),
                max_retries: args.ethereum_args.max_retries,
                retry_interval_ms: args.ethereum_args.retry_interval_ms,
                max_fee_per_gas: args.ethereum_args.max_fee_per_gas,
                max_priority_fee_per_gas: args.ethereum_args.max_priority_fee_per_gas,
            },
            http: EffectiveHttpConfig {
                address: args.web_server_address.clone(),
                token: web_server_token.to_string(),
            },
            storage: EffectiveStorageConfig { block_storage },
            proof_storage,
            options,
        };

        validate_effective_config(EffectiveConfig {
            prometheus_endpoint: args.prometheus_args.prometheus_endpoint.clone(),
            relayers: vec![relayer],
        })
    }

    pub fn from_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file {}", path.display()))?;
        Self::from_toml_str(&contents)
            .with_context(|| format!("Failed to parse config file {}", path.display()))
    }

    pub fn from_toml_str(contents: &str) -> anyhow::Result<Self> {
        let raw: RawConfig = toml::from_str(contents)?;
        raw.into_effective()
    }
}

#[derive(Deserialize)]
struct RawConfig {
    #[serde(default)]
    prometheus: RawPrometheusConfig,
    relayers: BTreeMap<String, RawRelayerConfig>,
}

#[derive(Default, Deserialize)]
struct RawPrometheusConfig {
    endpoint: Option<String>,
}

#[derive(Deserialize)]
struct RawRelayerConfig {
    priority: Option<i64>,
    gear: RawGearConfig,
    ethereum: RawEthereumConfig,
    genesis: RawGenesisConfig,
    http: RawHttpConfig,
    storage: RawStorageConfig,
    proof_storage: RawProofStorageConfig,
    #[serde(default)]
    gnark: RawGnarkConfig,
    #[serde(default)]
    options: RawOptionsConfig,
}

#[derive(Deserialize)]
struct RawGearConfig {
    endpoint: String,
    #[serde(default = "default_max_reconnect_attempts")]
    max_reconnect_attempts: u8,
}

#[derive(Deserialize)]
struct RawEthereumConfig {
    endpoint: String,
    message_queue_address: String,
    fee_payer: String,
    max_retries: Option<u32>,
    retry_interval_ms: Option<u64>,
    max_fee_per_gas: Option<u128>,
    max_priority_fee_per_gas: Option<u128>,
}

#[derive(Deserialize)]
struct RawGenesisConfig {
    authority_set_hash: String,
    authority_set_id: u64,
}

#[derive(Deserialize)]
struct RawHttpConfig {
    address: String,
    token: String,
}

#[derive(Deserialize)]
struct RawStorageConfig {
    block_storage: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum RawProofStorageConfig {
    #[serde(rename = "filesystem", alias = "file_system")]
    FileSystem { filesystem_path: PathBuf },
    #[serde(rename = "gear")]
    Gear {
        gear_fee_payer: String,
        config_dir: PathBuf,
    },
}

#[derive(Deserialize)]
struct RawGnarkConfig {
    #[serde(default = "default_gnark_data_path")]
    data_path: PathBuf,
}

impl Default for RawGnarkConfig {
    fn default() -> Self {
        Self {
            data_path: PathBuf::from(DEFAULT_GNARK_DATA_PATH),
        }
    }
}

#[derive(Default, Deserialize)]
struct RawOptionsConfig {
    confirmations_merkle_root: Option<u64>,
    start_authority_set_id: Option<u64>,
    bridging_payment_address: Option<String>,
    thread_count: Option<RawThreadCount>,
    critical_threshold: Option<String>,
    startup_sync_strategy: Option<String>,
    #[serde(default)]
    startup_sync_blocks: Vec<u32>,
    spike_window: Option<String>,
    spike_timeout: Option<String>,
    priority_spike_timeout: Option<String>,
    spike_threshold: Option<usize>,
    save_interval: Option<String>,
    check_interval: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum RawThreadCount {
    Auto(String),
    Manual(usize),
}

impl RawConfig {
    fn into_effective(self) -> anyhow::Result<EffectiveConfig> {
        if self.relayers.is_empty() {
            return Err(anyhow!("config must define at least one relayer"));
        }

        let mut relayers = Vec::with_capacity(self.relayers.len());
        for (id, relayer) in self.relayers {
            validate_relayer_id(&id)?;
            let priority = relayer
                .priority
                .ok_or_else(|| anyhow!("relayer {id}: priority is required"))?;

            validate_url(
                &relayer.gear.endpoint,
                &format!("relayer {id}: gear.endpoint"),
            )?;
            validate_url(
                &relayer.ethereum.endpoint,
                &format!("relayer {id}: ethereum.endpoint"),
            )?;
            let _ = decode_fixed_hex::<20>(
                &relayer.ethereum.message_queue_address,
                &id,
                "ethereum.message_queue_address",
            )?;
            let _ = decode_fixed_hex::<32>(&relayer.ethereum.fee_payer, &id, "ethereum.fee_payer")?;
            relayer
                .http
                .address
                .parse::<SocketAddr>()
                .with_context(|| format!("relayer {id}: http.address is invalid"))?;
            validate_non_empty(&relayer.http.token, &id, "http.token")?;
            validate_block_storage_path(&relayer.storage.block_storage, &id)?;
            validate_non_empty_path(&relayer.gnark.data_path, &id, "gnark.data_path")?;

            let thread_count = match relayer.options.thread_count {
                Some(raw) => Some(parse_thread_count(raw, &id)?).flatten(),
                None => Some(DEFAULT_COUNT_THREADS),
            };
            let critical_threshold =
                parse_critical_threshold(relayer.options.critical_threshold.as_deref(), &id)?;
            let startup_sync_blocks = relayer.options.startup_sync_blocks.clone();
            let startup_sync_strategy = parse_startup_sync_strategy(
                relayer.options.startup_sync_strategy.as_deref(),
                startup_sync_blocks.clone(),
                &id,
            )?;
            let spike_window = parse_duration(
                relayer.options.spike_window.as_deref(),
                "15m",
                &id,
                "options.spike_window",
            )?;
            let spike_timeout = parse_duration(
                relayer.options.spike_timeout.as_deref(),
                "30m",
                &id,
                "options.spike_timeout",
            )?;
            let priority_spike_timeout = parse_duration(
                relayer.options.priority_spike_timeout.as_deref(),
                "10m",
                &id,
                "options.priority_spike_timeout",
            )?;
            if priority_spike_timeout > spike_timeout {
                return Err(anyhow!(
                    "relayer {id}: options.priority_spike_timeout must not exceed options.spike_timeout"
                ));
            }

            let save_interval = parse_duration(
                relayer.options.save_interval.as_deref(),
                "30m",
                &id,
                "options.save_interval",
            )?;
            let check_interval = parse_duration(
                relayer.options.check_interval.as_deref(),
                "30s",
                &id,
                "options.check_interval",
            )?;

            let proof_storage = match relayer.proof_storage {
                RawProofStorageConfig::FileSystem { filesystem_path } => {
                    validate_non_empty_path(
                        &filesystem_path,
                        &id,
                        "proof_storage.filesystem_path",
                    )?;
                    EffectiveProofStorageConfig::FileSystem {
                        path: filesystem_path,
                    }
                }
                RawProofStorageConfig::Gear {
                    gear_fee_payer,
                    config_dir,
                } => EffectiveProofStorageConfig::Gear {
                    fee_payer: {
                        validate_non_empty(&gear_fee_payer, &id, "proof_storage.gear_fee_payer")?;
                        gear_fee_payer
                    },
                    config_dir: {
                        validate_non_empty_path(&config_dir, &id, "proof_storage.config_dir")?;
                        config_dir
                    },
                },
            };

            let options = build_options(OptionSource {
                relayer_id: &id,
                priority,
                confirmations_merkle_root: relayer.options.confirmations_merkle_root,
                start_authority_set_id: relayer.options.start_authority_set_id,
                bridging_payment_address: relayer.options.bridging_payment_address.as_deref(),
                thread_count,
                critical_threshold,
                startup_sync_strategy,
                startup_sync_blocks,
                spike_window,
                spike_timeout,
                priority_spike_timeout,
                spike_threshold: relayer.options.spike_threshold.unwrap_or(8),
                save_interval,
                check_interval,
                gnark_data_path: relayer.gnark.data_path.clone(),
                authority_set_hash: &relayer.genesis.authority_set_hash,
                authority_set_id: relayer.genesis.authority_set_id,
            })?;

            relayers.push(EffectiveRelayerConfig {
                id,
                priority,
                gear: EffectiveGearConfig {
                    endpoint: relayer.gear.endpoint,
                    max_reconnect_attempts: relayer.gear.max_reconnect_attempts,
                },
                ethereum: EffectiveEthereumConfig {
                    endpoint: relayer.ethereum.endpoint,
                    message_queue_address: relayer.ethereum.message_queue_address,
                    fee_payer: relayer.ethereum.fee_payer,
                    max_retries: relayer.ethereum.max_retries,
                    retry_interval_ms: relayer.ethereum.retry_interval_ms,
                    max_fee_per_gas: relayer.ethereum.max_fee_per_gas,
                    max_priority_fee_per_gas: relayer.ethereum.max_priority_fee_per_gas,
                },
                http: EffectiveHttpConfig {
                    address: relayer.http.address,
                    token: relayer.http.token,
                },
                storage: EffectiveStorageConfig {
                    block_storage: relayer.storage.block_storage,
                },
                proof_storage,
                options,
            });
        }

        validate_effective_config(EffectiveConfig {
            prometheus_endpoint: self
                .prometheus
                .endpoint
                .unwrap_or_else(|| DEFAULT_PROMETHEUS_ENDPOINT.to_string()),
            relayers,
        })
    }
}

struct OptionSource<'a> {
    relayer_id: &'a str,
    priority: i64,
    confirmations_merkle_root: Option<u64>,
    start_authority_set_id: Option<u64>,
    bridging_payment_address: Option<&'a str>,
    thread_count: Option<usize>,
    critical_threshold: cli::CriticalThreshold,
    startup_sync_strategy: cli::StartupSyncStrategy,
    startup_sync_blocks: Vec<u32>,
    spike_window: Duration,
    spike_timeout: Duration,
    priority_spike_timeout: Duration,
    spike_threshold: usize,
    save_interval: Duration,
    check_interval: Duration,
    gnark_data_path: PathBuf,
    authority_set_hash: &'a str,
    authority_set_id: u64,
}

fn build_options(source: OptionSource<'_>) -> anyhow::Result<MerkleRootRelayerOptions> {
    let startup_sync_strategy = match source.startup_sync_strategy {
        cli::StartupSyncStrategy::CriticalThreshold => StartupSyncStrategy::CriticalThreshold,
        cli::StartupSyncStrategy::SkipCatchUp => StartupSyncStrategy::SkipCatchUp,
        cli::StartupSyncStrategy::Blocks => {
            if source.startup_sync_blocks.is_empty() {
                return Err(anyhow!(
                    "relayer {}: startup_sync_blocks must be provided when startup_sync_strategy=blocks",
                    source.relayer_id
                ));
            }
            StartupSyncStrategy::Blocks(source.startup_sync_blocks.clone())
        }
    };

    if !matches!(startup_sync_strategy, StartupSyncStrategy::Blocks(_))
        && !source.startup_sync_blocks.is_empty()
    {
        return Err(anyhow!(
            "relayer {}: startup_sync_blocks can only be used when startup_sync_strategy=blocks",
            source.relayer_id
        ));
    }

    let critical_threshold = match source.critical_threshold {
        cli::CriticalThreshold::Timeout(duration) => {
            if duration < Duration::from_secs(3) {
                return Err(anyhow!(
                    "relayer {}: critical_threshold duration must be at least 3 seconds",
                    source.relayer_id
                ));
            }
            CriticalThreshold::Timeout((duration.as_secs() / 3) as u32)
        }
        cli::CriticalThreshold::AuthoritySetChange => CriticalThreshold::AuthoritySetChange,
    };

    let authority_set_hash = decode_fixed_hex::<BLAKE2_DIGEST_SIZE>(
        source.authority_set_hash,
        source.relayer_id,
        "genesis.authority_set_hash",
    )?;

    let bridging_payment_address = source
        .bridging_payment_address
        .map(|value| {
            decode_fixed_hex::<32>(value, source.relayer_id, "options.bridging_payment_address")
                .map(H256::from)
        })
        .transpose()?;

    Ok(MerkleRootRelayerOptions {
        relayer_id: source.relayer_id.to_string(),
        priority: source.priority,
        spike_config: SpikeConfig {
            timeout: source.spike_timeout,
            priority_timeout: source.priority_spike_timeout,
            window: source.spike_window,
            threshold: source.spike_threshold,
        },
        check_interval: source.check_interval,
        save_interval: source.save_interval,
        genesis_config: GenesisConfig {
            authority_set_hash,
            authority_set_id: source.authority_set_id,
        },
        last_sealed: source.start_authority_set_id,
        confirmations: source
            .confirmations_merkle_root
            .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
        count_thread: source.thread_count,
        bridging_payment_address,
        critical_threshold,
        startup_sync_strategy,
        gnark_data_path: source.gnark_data_path,
        shared_authority_set_sync: None,
    })
}

fn validate_effective_config(config: EffectiveConfig) -> anyhow::Result<EffectiveConfig> {
    let mut http_addresses = HashSet::new();
    let mut block_storage_paths = HashSet::new();
    let mut proof_storage_paths = HashSet::new();

    for relayer in &config.relayers {
        if !http_addresses.insert(relayer.http.address.clone()) {
            return Err(anyhow!(
                "http.address {} is used by more than one relayer",
                relayer.http.address
            ));
        }
        if !block_storage_paths.insert(relayer.storage.block_storage.clone()) {
            return Err(anyhow!(
                "storage.block_storage {} is used by more than one relayer",
                relayer.storage.block_storage.display()
            ));
        }
        let proof_path = match &relayer.proof_storage {
            EffectiveProofStorageConfig::FileSystem { path } => path,
            EffectiveProofStorageConfig::Gear { config_dir, .. } => config_dir,
        };
        if !proof_storage_paths.insert(proof_path.clone()) {
            return Err(anyhow!(
                "proof storage path {} is used by more than one relayer",
                proof_path.display()
            ));
        }
    }

    Ok(config)
}

fn validate_relayer_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(anyhow!(
            "invalid relayer id {id:?}; expected [A-Za-z0-9_-]+"
        ));
    }
    Ok(())
}

fn decode_fixed_hex<const LEN: usize>(
    value: &str,
    relayer_id: &str,
    field: &str,
) -> anyhow::Result<[u8; LEN]> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(value)
        .with_context(|| format!("relayer {relayer_id}: {field} is invalid hex"))?;
    bytes.try_into().map_err(|got: Vec<u8>| {
        anyhow!(
            "relayer {relayer_id}: {field} has wrong length. Expected {}, got {}",
            LEN,
            got.len()
        )
    })
}

fn validate_url(value: &str, field: &str) -> anyhow::Result<()> {
    Url::parse(value).with_context(|| format!("{field} must be a valid URL"))?;
    Ok(())
}

fn validate_block_storage_path(path: &Path, relayer_id: &str) -> anyhow::Result<()> {
    if path.as_os_str().is_empty() {
        return Err(anyhow!(
            "relayer {relayer_id}: storage.block_storage is empty"
        ));
    }
    if path.exists() && path.is_dir() {
        return Err(anyhow!(
            "relayer {relayer_id}: storage.block_storage must be a file path, got directory {}",
            path.display()
        ));
    }
    Ok(())
}

fn validate_non_empty(value: &str, relayer_id: &str, field: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        return Err(anyhow!("relayer {relayer_id}: {field} must not be empty"));
    }
    Ok(())
}

fn validate_non_empty_path(path: &Path, relayer_id: &str, field: &str) -> anyhow::Result<()> {
    if path.as_os_str().is_empty() {
        return Err(anyhow!("relayer {relayer_id}: {field} must not be empty"));
    }
    Ok(())
}

fn parse_thread_count(raw: RawThreadCount, relayer_id: &str) -> anyhow::Result<Option<usize>> {
    match raw {
        RawThreadCount::Manual(count) if count > 0 => Ok(Some(count)),
        RawThreadCount::Manual(_) => Err(anyhow!(
            "relayer {relayer_id}: options.thread_count must be positive or \"auto\""
        )),
        RawThreadCount::Auto(value) if value.eq_ignore_ascii_case("auto") => Ok(None),
        RawThreadCount::Auto(value) => Err(anyhow!(
            "relayer {relayer_id}: options.thread_count must be a positive integer or \"auto\", got {value:?}"
        )),
    }
}

fn parse_critical_threshold(
    value: Option<&str>,
    relayer_id: &str,
) -> anyhow::Result<cli::CriticalThreshold> {
    cli::parse_critical_threshold(value.unwrap_or("14h"))
        .with_context(|| format!("relayer {relayer_id}: options.critical_threshold is invalid"))
}

fn parse_startup_sync_strategy(
    value: Option<&str>,
    blocks: Vec<u32>,
    relayer_id: &str,
) -> anyhow::Result<cli::StartupSyncStrategy> {
    let strategy = cli::parse_startup_sync_strategy(value.unwrap_or("critical-threshold"))
        .with_context(|| {
            format!("relayer {relayer_id}: options.startup_sync_strategy is invalid")
        })?;

    if strategy == cli::StartupSyncStrategy::Blocks && blocks.is_empty() {
        return Err(anyhow!(
            "relayer {relayer_id}: options.startup_sync_blocks must be provided when options.startup_sync_strategy=blocks"
        ));
    }
    if strategy != cli::StartupSyncStrategy::Blocks && !blocks.is_empty() {
        return Err(anyhow!(
            "relayer {relayer_id}: options.startup_sync_blocks can only be used when options.startup_sync_strategy=blocks"
        ));
    }

    Ok(strategy)
}

fn parse_duration(
    value: Option<&str>,
    default: &str,
    relayer_id: &str,
    field: &str,
) -> anyhow::Result<Duration> {
    let duration = humantime::parse_duration(value.unwrap_or(default))
        .with_context(|| format!("relayer {relayer_id}: {field} is invalid"))?;
    if duration.is_zero() {
        return Err(anyhow!("relayer {relayer_id}: {field} must be positive"));
    }
    Ok(duration)
}

fn required<'a>(value: Option<&'a str>, field: &str) -> anyhow::Result<&'a str> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Missing {field}"))
}

fn default_max_reconnect_attempts() -> u8 {
    3
}

fn default_gnark_data_path() -> PathBuf {
    PathBuf::from(DEFAULT_GNARK_DATA_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> String {
        format!(
            r#"
[prometheus]
endpoint = "127.0.0.1:9900"

[relayers.mainnet]
priority = 100

[relayers.mainnet.gear]
endpoint = "wss://gear.example"
max_reconnect_attempts = 4

[relayers.mainnet.ethereum]
endpoint = "https://eth.example"
message_queue_address = "0x1111111111111111111111111111111111111111"
fee_payer = "0x{}"
max_retries = 5
retry_interval_ms = 2000
max_fee_per_gas = 2000000000
max_priority_fee_per_gas = 500000000

[relayers.mainnet.genesis]
authority_set_hash = "0x{}"
authority_set_id = 42

[relayers.mainnet.http]
address = "127.0.0.1:8443"
token = "secret"

[relayers.mainnet.storage]
block_storage = "/tmp/mainnet-blocks.json"

[relayers.mainnet.proof_storage]
kind = "filesystem"
filesystem_path = "/tmp/mainnet-proofs"

[relayers.mainnet.gnark]
data_path = "/tmp/mainnet-gnark"

[relayers.mainnet.options]
confirmations_merkle_root = 8
thread_count = "auto"
critical_threshold = "14h"
startup_sync_strategy = "critical-threshold"
spike_window = "15m"
spike_timeout = "30m"
priority_spike_timeout = "10m"
spike_threshold = 8
save_interval = "30m"
check_interval = "30s"
"#,
            "22".repeat(32),
            "33".repeat(32),
        )
    }

    fn config_error(contents: &str) -> String {
        match EffectiveConfig::from_toml_str(contents) {
            Ok(_) => panic!("config unexpectedly parsed successfully"),
            Err(err) => err.to_string(),
        }
    }

    fn second_relayer(http: &str, block_storage: &str, proof_storage: &str) -> String {
        format!(
            r#"
[relayers.testnet]
priority = 50

[relayers.testnet.gear]
endpoint = "wss://gear2.example"

[relayers.testnet.ethereum]
endpoint = "https://eth2.example"
message_queue_address = "0x2222222222222222222222222222222222222222"
fee_payer = "0x{}"

[relayers.testnet.genesis]
authority_set_hash = "0x{}"
authority_set_id = 43

[relayers.testnet.http]
address = "{http}"
token = "secret2"

[relayers.testnet.storage]
block_storage = "{block_storage}"

[relayers.testnet.proof_storage]
kind = "filesystem"
filesystem_path = "{proof_storage}"

[relayers.testnet.gnark]
data_path = "/tmp/testnet-gnark"
"#,
            "44".repeat(32),
            "55".repeat(32),
        )
    }

    #[test]
    fn parses_valid_config() {
        let config = EffectiveConfig::from_toml_str(&valid_config()).unwrap();
        assert_eq!(config.prometheus_endpoint, "127.0.0.1:9900");
        assert_eq!(config.relayers.len(), 1);
        let relayer = &config.relayers[0];
        assert_eq!(relayer.id, "mainnet");
        assert_eq!(relayer.options.relayer_id, "mainnet");
        assert_eq!(relayer.priority, 100);
        assert_eq!(relayer.gear.endpoint, "wss://gear.example");
        assert_eq!(relayer.gear.max_reconnect_attempts, 4);
        assert_eq!(relayer.ethereum.max_retries, Some(5));
        assert_eq!(
            relayer.options.gnark_data_path,
            PathBuf::from("/tmp/mainnet-gnark")
        );
        assert_eq!(relayer.options.count_thread, None);
    }

    #[test]
    fn defaults_gnark_data_path_to_data() {
        let config = valid_config().replace(
            "\n[relayers.mainnet.gnark]\ndata_path = \"/tmp/mainnet-gnark\"\n",
            "\n",
        );

        let config = EffectiveConfig::from_toml_str(&config).unwrap();

        assert_eq!(
            config.relayers[0].options.gnark_data_path,
            PathBuf::from("data")
        );
    }

    #[test]
    fn rejects_empty_gnark_data_path() {
        let config =
            valid_config().replace("data_path = \"/tmp/mainnet-gnark\"", "data_path = \"\"");

        let err = config_error(&config);

        assert!(err.contains("gnark.data_path"));
    }

    #[test]
    fn parses_multiple_relayers_and_priorities() {
        let config = valid_config()
            + &second_relayer(
                "127.0.0.1:8444",
                "/tmp/testnet-blocks.json",
                "/tmp/testnet-proofs",
            );
        let config = EffectiveConfig::from_toml_str(&config).unwrap();
        assert_eq!(config.relayers.len(), 2);
        assert_eq!(config.relayers[0].id, "mainnet");
        assert_eq!(config.relayers[0].options.relayer_id, "mainnet");
        assert_eq!(config.relayers[0].priority, 100);
        assert_eq!(config.relayers[1].id, "testnet");
        assert_eq!(config.relayers[1].options.relayer_id, "testnet");
        assert_eq!(config.relayers[1].priority, 50);
    }

    #[test]
    fn example_config_parses() {
        let config = EffectiveConfig::from_toml_str(include_str!("../config.toml.example"))
            .expect("example config must stay in sync with the TOML schema");
        assert_eq!(config.relayers.len(), 2);
        assert_eq!(config.relayers[0].id, "mainnet");
        assert_eq!(config.relayers[1].id, "testnet");
    }

    #[test]
    fn rejects_empty_relayers() {
        let err = config_error("[prometheus]\nendpoint = \"127.0.0.1:9900\"\n");
        assert!(err.contains("relayers"));
    }

    #[test]
    fn rejects_missing_priority() {
        let config = valid_config().replace("priority = 100", "");
        let err = config_error(&config);
        assert!(err.contains("priority is required"));
    }

    #[test]
    fn rejects_duplicate_http_address() {
        let config = valid_config()
            + &second_relayer(
                "127.0.0.1:8443",
                "/tmp/testnet-blocks.json",
                "/tmp/testnet-proofs",
            );
        let err = config_error(&config);
        assert!(err.contains("http.address"));
    }

    #[test]
    fn rejects_duplicate_block_storage() {
        let config = valid_config()
            + &second_relayer(
                "127.0.0.1:8444",
                "/tmp/mainnet-blocks.json",
                "/tmp/testnet-proofs",
            );
        let err = config_error(&config);
        assert!(err.contains("storage.block_storage"));
    }

    #[test]
    fn rejects_duplicate_proof_storage() {
        let config = valid_config()
            + &second_relayer(
                "127.0.0.1:8444",
                "/tmp/testnet-blocks.json",
                "/tmp/mainnet-proofs",
            );
        let err = config_error(&config);
        assert!(err.contains("proof storage path"));
    }

    #[test]
    fn rejects_invalid_ethereum_fee_payer() {
        let config = valid_config().replace(
            &format!("fee_payer = \"0x{}\"", "22".repeat(32)),
            "fee_payer = \"\"",
        );
        let err = config_error(&config);
        assert!(err.contains("ethereum.fee_payer"));
    }

    #[test]
    fn rejects_empty_http_token() {
        let config = valid_config().replace("token = \"secret\"", "token = \"\"");
        let err = config_error(&config);
        assert!(err.contains("http.token"));
    }

    #[test]
    fn rejects_empty_filesystem_proof_storage_path() {
        let config = valid_config().replace(
            "filesystem_path = \"/tmp/mainnet-proofs\"",
            "filesystem_path = \"\"",
        );
        let err = config_error(&config);
        assert!(err.contains("proof_storage.filesystem_path"));
    }

    #[test]
    fn rejects_invalid_relayer_id() {
        let config = valid_config().replace("[relayers.mainnet", "[relayers.\"bad.id\"");
        let err = config_error(&config);
        assert!(err.contains("invalid relayer id"));
    }

    #[test]
    fn rejects_startup_blocks_without_blocks_strategy() {
        let config = valid_config()
            + r#"
startup_sync_blocks = [1, 2]
"#;
        let err = config_error(&config);
        assert!(err.contains("startup_sync_blocks"));
    }

    #[test]
    fn rejects_tiny_critical_threshold() {
        let config = valid_config().replace(
            "critical_threshold = \"14h\"",
            "critical_threshold = \"2s\"",
        );
        let err = config_error(&config);
        assert!(err.contains("at least 3 seconds"));
    }

    #[test]
    fn rejects_zero_duration() {
        let config = valid_config().replace("check_interval = \"30s\"", "check_interval = \"0s\"");
        let err = config_error(&config);
        assert!(err.contains("options.check_interval"));
        assert!(err.contains("positive"));
    }

    #[test]
    fn rejects_zero_thread_count() {
        let config = valid_config().replace("thread_count = \"auto\"", "thread_count = 0");
        let err = config_error(&config);
        assert!(err.contains("options.thread_count"));
        assert!(err.contains("positive"));
    }

    #[test]
    fn rejects_priority_timeout_greater_than_regular_timeout() {
        let config = valid_config().replace(
            "priority_spike_timeout = \"10m\"",
            "priority_spike_timeout = \"40m\"",
        );
        let err = config_error(&config);
        assert!(err.contains("priority_spike_timeout"));
    }
}
