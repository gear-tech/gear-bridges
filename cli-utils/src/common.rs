use clap::Args;
use std::path::PathBuf;

#[derive(Args, Clone, Debug)]
pub struct BlockStorageArgs {
    /// Path to the block storage. For merkle root relayer this should be
    /// a path to JSON file where unprocessed blocks will be saved. For
    /// other relayers, this should be a path to a directory where blocks
    /// are stored in individual files.
    #[arg(long = "block-storage", env = "GEAR_BLOCK_STORAGE")]
    pub path: Option<PathBuf>,
}

#[derive(Args, Clone, Debug)]
pub struct GenesisConfigArgs {
    /// Authority set hash used in genesis config
    #[arg(long, env = "GENESIS_CONFIG_AUTHORITY_SET_HASH")]
    pub authority_set_hash: String,
    /// Authority set id used in genesis config
    #[arg(long, env = "GENESIS_CONFIG_AUTHORITY_SET_ID")]
    pub authority_set_id: u64,
}
