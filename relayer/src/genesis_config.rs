use gear_rpc_client::GearApi;
use prover::proving::GenesisConfig;
use serde::{Deserialize, Serialize};

pub async fn fetch_from_chain(
    gear_api: GearApi,
    block: Option<u32>,
    write_to_file: bool,
) -> anyhow::Result<()> {
    let block = match block {
        Some(bn) => Some(
            gear_api
                .block_number_to_hash(bn)
                .await
                .expect("Failed to fetch block hash by number"),
        ),
        None => None,
    };

    let state = gear_api.authority_set_state(block).await.unwrap();

    println!("Authority set id: {}", state.authority_set_id);
    println!(
        "Authority set hash: {}",
        hex::encode(&state.authority_set_hash)
    );

    if write_to_file {
        let config = GenesisConfig {
            authority_set_id: state.authority_set_id,
            authority_set_hash: state.authority_set_hash,
        };

        write_config_to_file(config);
    }

    Ok(())
}

#[derive(Deserialize, Serialize)]
struct GenesisConfigToml {
    authority_set_id: u64,
    authority_set_hash: String,
}

pub fn load_from_file() -> GenesisConfig {
    let data =
        std::fs::read_to_string("./GenesisConfig.toml").expect("Genesis config is not found");
    let config: GenesisConfigToml =
        toml::from_str(&data).expect("Wrong GenesisConfig.toml file structure");

    let hash = hex::decode(&config.authority_set_hash)
        .expect("Incorrect format for authority set hash: hex-encoded hash is expected");
    let hash = hash
        .try_into()
        .expect("Incorrect format for authority set hash: wrong length");

    GenesisConfig {
        authority_set_id: config.authority_set_id,
        authority_set_hash: hash,
    }
}

fn write_config_to_file(config: GenesisConfig) {
    let config = GenesisConfigToml {
        authority_set_id: config.authority_set_id,
        authority_set_hash: hex::encode(&config.authority_set_hash),
    };

    let data = toml::to_string(&config).expect("Failed to serialize config");

    std::fs::write("./GenesisConfig.toml", data).expect("Failed to write genesis config to file");
}
