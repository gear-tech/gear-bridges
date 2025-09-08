use parity_scale_codec::Encode;
use gear_rpc_client::{dto, GearApi};
use num::BigUint;
use primitive_types::H256;
use prometheus::{core::Collector, HistogramOpts, HistogramVec};
use prover::proving::{
    self, BlockFinality, BranchNodeData, GenesisConfig, PreCommit, ProofWithCircuitData,
    StorageInclusion,
};
use serde::{Deserialize, Serialize};
use std::{str::FromStr, thread, time::Instant};
use utils_prometheus::MeteredService;

pub struct Metrics;

impl MeteredService for Metrics {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        let proving_time: Box<dyn Collector> = Box::from(PROVING_TIME.clone());
        [proving_time]
    }
}

lazy_static::lazy_static!(
    static ref PROVING_TIME: HistogramVec = HistogramVec::new(
            HistogramOpts::new("proving_time", "ZK circuits proving time"),
            &["circuit"],
        ).unwrap();
);

pub async fn prove_genesis(
    gear_api: &GearApi,
    genesis_config: GenesisConfig,

    count_thread: Option<usize>,
) -> anyhow::Result<ProofWithCircuitData> {
    log::info!(
        "Proving genesis authority set change {} -> {}",
        genesis_config.authority_set_id,
        genesis_config.authority_set_id + 1
    );

    let (block, current_epoch_block_finality) = gear_api
        .fetch_finality_proof_for_session(genesis_config.authority_set_id)
        .await?;

    let next_validator_set_inclusion_proof = gear_api
        .fetch_next_session_keys_inclusion_proof(block)
        .await?;

    let next_validator_set_storage_data = next_validator_set_inclusion_proof.stored_data.clone();
    let next_validator_set_inclusion_proof =
        parse_rpc_inclusion_proof(next_validator_set_inclusion_proof);

    log::trace!("Start to prove genesis");

    let now = Instant::now();
    let timer = PROVING_TIME.with_label_values(&["genesis"]).start_timer();

    let handler = thread::spawn(move || {
        proving::prove_genesis(
            parse_rpc_block_finality_proof(current_epoch_block_finality, count_thread),
            genesis_config,
            next_validator_set_inclusion_proof,
            next_validator_set_storage_data,
        )
    });

    let proof = handler
        .join()
        .expect("prover::proving::prove_genesis handle should be joined");

    timer.stop_and_record();
    log::info!("Genesis prove time: {}ms", now.elapsed().as_millis());

    Ok(proof)
}

pub async fn prove_validator_set_change(
    gear_api: &GearApi,
    previous_proof: ProofWithCircuitData,
    previous_authority_set_id: u64,

    count_thread: Option<usize>,
) -> anyhow::Result<ProofWithCircuitData> {
    log::info!(
        "Proving authority set change {} -> {}",
        previous_authority_set_id,
        previous_authority_set_id + 1
    );

    let (block, current_epoch_block_finality) = gear_api
        .fetch_finality_proof_for_session(previous_authority_set_id)
        .await?;

    let next_validator_set_inclusion_proof = gear_api
        .fetch_next_session_keys_inclusion_proof(block)
        .await?;
    let next_validator_set_storage_data = next_validator_set_inclusion_proof.stored_data.clone();
    let next_validator_set_inclusion_proof =
        parse_rpc_inclusion_proof(next_validator_set_inclusion_proof);

    let now = Instant::now();
    let timer = PROVING_TIME
        .with_label_values(&["validator_set_change"])
        .start_timer();

    let handler = thread::spawn(move || {
        proving::prove_validator_set_change(
            previous_proof,
            parse_rpc_block_finality_proof(current_epoch_block_finality, count_thread),
            next_validator_set_inclusion_proof,
            next_validator_set_storage_data,
        )
    });

    let proof = handler
        .join()
        .expect("proving::prove_validator_set_change handle should be joined");

    timer.stop_and_record();
    log::info!("Recursive prove time: {}ms", now.elapsed().as_millis());

    Ok(proof)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalProof {
    pub proof: Vec<u8>,
    pub block_number: u32,
    pub merkle_root: [u8; 32],
}

impl FinalProof {
    pub fn from_proof_and_public_inputs(proof: String, public_inputs: [BigUint; 2]) -> Self {
        // data layout:
        // root[0] root[1] root[2] root[3] root[4] root[5]
        // root[6] root[7] block_n   pad     pad     pad

        fn pad_pi(pi: &BigUint) -> Vec<u8> {
            const TARGET_LEN: usize = 24;

            let mut pi = pi.to_bytes_be();
            assert!(pi.len() <= TARGET_LEN);
            let mut padding = vec![0; TARGET_LEN - pi.len()];
            padding.append(&mut pi);
            padding
        }

        let pi_0 = pad_pi(&public_inputs[0]);
        let pi_1 = pad_pi(&public_inputs[1]);

        let first_root_part = &pi_0[..];
        let second_root_part = &pi_1[..8];

        let root_bytes = [first_root_part, second_root_part].concat();
        let block_number = &pi_1[8..12];

        assert_eq!(&proof[..2], "0x");

        Self {
            proof: hex::decode(&proof[2..]).expect("Got invalid proof string from gnark prover"),
            block_number: u32::from_be_bytes(
                block_number
                    .try_into()
                    .expect("Wrong amount of bytes to build block number"),
            ),
            merkle_root: root_bytes
                .try_into()
                .expect("Wrong amount of bytes to build merkle tree root"),
        }
    }
}

pub async fn prove_final(
    gear_api: &GearApi,
    previous_proof: ProofWithCircuitData,
    genesis_config: GenesisConfig,
    at_block: H256,

    count_thread: Option<usize>,
) -> anyhow::Result<FinalProof> {
    let (block, block_finality) = gear_api.fetch_finality_proof(at_block).await?;
    prove_final_with_block_finality(
        gear_api,
        previous_proof,
        genesis_config,
        (block, block_finality),
        count_thread,
    )
    .await
}

pub async fn prove_final_with_block_finality(
    gear_api: &GearApi,
    previous_proof: ProofWithCircuitData,
    genesis_config: GenesisConfig,
    (block, block_finality): (H256, dto::BlockFinalityProof),

    count_thread: Option<usize>,
) -> anyhow::Result<FinalProof> {
    let sent_message_inclusion_proof = gear_api.fetch_sent_message_inclusion_proof(block).await?;

    let message_contents = sent_message_inclusion_proof.stored_data.clone();
    let sent_message_inclusion_proof = parse_rpc_inclusion_proof(sent_message_inclusion_proof);

    let now = Instant::now();
    let timer = PROVING_TIME.with_label_values(&["final"]).start_timer();
    let now = Instant::now();

    let handler = thread::spawn(move || {
        let proof = proving::prove_message_sent(
            previous_proof,
            parse_rpc_block_finality_proof(block_finality, count_thread),
            genesis_config,
            sent_message_inclusion_proof,
            message_contents,
        );

        gnark::prove_circuit(&proof)
    });

    let proof = handler
        .join()
        .expect("proving::prove_message_sent & gnark handle should be joined");

    log::info!("prove_final_with_block_finality took: {}ms", now.elapsed().as_millis());
    timer.stop_and_record();
    log::info!("Final prove time: {}ms", now.elapsed().as_millis());

    let public_inputs: [_; 2] = proof
        .public_inputs
        .try_into()
        .expect("Got wrong public input count from gnark prover");

    let public_inputs = public_inputs
        .map(|s| BigUint::from_str(&s).expect("Got wrong public input format from ganrk"));

    Ok(FinalProof::from_proof_and_public_inputs(
        proof.proof,
        public_inputs,
    ))
}

fn parse_rpc_inclusion_proof(proof: dto::StorageInclusionProof) -> StorageInclusion {
    let address_nibbles = proof
        .address
        .into_iter()
        .flat_map(|byte| [(byte & 0b11110000) >> 4, byte & 0b00001111])
        .collect();

    StorageInclusion {
        block_header_data: proof.block_header,
        branch_node_data: proof
            .branch_nodes_data
            .into_iter()
            .rev()
            .map(|d| BranchNodeData {
                data: d.data,
                child_nibble: d.target_child,
            })
            .collect(),
        leaf_node_data: proof.leaf_node_data,
        address_nibbles,
    }
}

fn parse_rpc_block_finality_proof(
    proof: dto::BlockFinalityProof,
    count_thread: Option<usize>,
) -> BlockFinality {
    BlockFinality {
        validator_set: proof.validator_set,
        pre_commits: proof
            .pre_commits
            .into_iter()
            .map(|pc| PreCommit {
                public_key: pc.public_key,
                signature: pc.signature,
            })
            .collect(),
        message: proof
            .message
            .try_into()
            .expect("Unexpected GRANDPA message length"),
        count_thread,
    }
}

pub mod gnark {
    use core::ffi::c_char;
    use std::ffi::{CStr, CString};

    use prover::proving::ExportedProofWithCircuitData;
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, Serialize)]
    pub struct ProveResult {
        pub proof: String,
        pub public_inputs: Vec<String>,
    }

    extern "C" {
        fn prove(circuit_data: *const c_char) -> *const c_char;
    }

    pub fn prove_circuit(s: &ExportedProofWithCircuitData) -> ProveResult {
        let serialized = serde_json::to_string(s).expect("Failed to serialize data");
        let c_string = CString::new(serialized).expect("CString::new failed");
        let result = unsafe {
            let result_ptr = prove(c_string.as_ptr());
            if result_ptr.is_null() {
                panic!("prove returned null pointer");
            }
            // Convert the result pointer to a Rust string.
            let result_cstr = CStr::from_ptr(result_ptr);
            let result_str = result_cstr.to_str().expect("Invalid UTF-8 sequence");
            let owned = result_str.to_owned();
            libc::free(result_ptr as *mut libc::c_void);
            owned
        };

        serde_json::from_str(&result).expect("Got wrong output from gnark prover")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parity_scale_codec::Decode;

    fn get_thread_count() -> usize {
        const DEFAULT: usize = 12;

        let Ok(thread_count) = std::env::var("THREAD_COUNT") else {
            log::debug!(r#""THREAD_COUNT" is not set. Use default value ({DEFAULT})."#);

            return DEFAULT;
        };

        let Ok(thread_count) = thread_count.parse::<usize>() else {
            log::debug!(r#""THREAD_COUNT" is not a number. Use default value ({DEFAULT})."#);

            return DEFAULT;
        };

        thread_count
    }

    #[test]
    fn test_60() {
        // const FILE_NAME: &str = "/tmp/test-60--efd1229d-2765-47da-947d-73678fd708eb";

        let _ = pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Info)
            .format_target(false)
            .format_timestamp_secs()
            .parse_default_env()
            .try_init();

        let thread_count = get_thread_count();

        // log::info!(r#"Waiting for "{FILE_NAME}""#);

        // while !std::fs::exists(FILE_NAME).unwrap() {
        //     std::thread::sleep(std::time::Duration::from_millis(10));
        // }

        // std::fs::remove_file(FILE_NAME).unwrap();

        let genesis_config = GenesisConfig { authority_set_id: 4, authority_set_hash: [156, 27, 28, 75, 28, 238, 10, 160, 36, 252, 179, 100, 186, 171, 78, 50, 116, 205, 64, 230, 159, 203, 123, 205, 32, 98, 171, 6, 42, 244, 23, 150] };

        let previous_proof = {
            // let block = Decode::decode(&mut &hex_literal::hex!("fb05571ff76e3864698d47c66a25252b4dd6eba9f65ff8f5978b0359a2395b41")[..]).unwrap();
            let current_epoch_block_finality = Decode::decode(&mut &hex_literal::hex!("f0445f7d52a0405ebf12f1b69cf1e7637c6f04a7d67e1000df3357894e5bdb294dc2252920ec60963094a935b6cfc483187d4092fa1d4af6c17a8fa93d90ffb826c351f301f71458df98b139511ecd8a9e4e0d1b18471a4075ac6ae8c02443bb2e983bd8c44f9d09d1f17bfc97fc494f0100d054f50c927cf35fd71f0df7788a7fe0c5b5ec1b83df6126744a6bff0cd76ab7c046997f1d7e0d6b4ce62133b3b35db5af6489c3676b81365bd07bbc40c41833c8dca8cf433287805fcd3bbb0d35eee6cd4fa4fde4ab4313a607c588a97d4dcea63fd1265cee969d43148fcfcba89f1aee03d498efa7f271afabdd9997e1c393bfa345a88525e6dc66bea95904c2164f5f8c8ecb586670a7935c4395eb85d3072311f5b1942e6bc2d658e7b835dc154bf75538620f9ca48a473526c95e4fc0c68f2ab37e3cf3f86db4e70145b85b18f19aa7b82162a73d6685224ea61bccacb80a0db0bc172b560edd9969bd06021e0a5cd87a6f76c0f6ce984a92ffa0379e01e0c9b87c7bdbed4aea25b76b3f062176d3a063b9d141719b87c9dae10a52a94b5f8818d1bd12f41308a7654bb89045c56ba044a5f3e9e4541b1de73b75db78c22838eca8a7bffe4f9507dcfdae35783ff89a98ee0654affbf2fd1c97d9cb4aab58e96855ec13bea0a88c06ce4ac05f8e6c6bac12f96d9b8ac1e021e028ad0d20a0056fb4c0df6b8e0c45ef53779fe12c5950454cc20d515cda4651ee54756889d8d63b8a80f640402779a8ccf03ecc06fcca0083a3bd75816f7a3a648d60a93188802bafa3d11653d66a9dae47c57f445f86c96680f0a9a243a8d14e14f718ac5f801b713df816dc1a0b7481e2b372fb44a7e8d87632b76beeddf1cbee936661c8a537c5bc2d1e71605b18592e16f924c87f0e8b3f708353c643c1c09b9ac58bfe315f806f40bf8b2eb2e4ec35908356b818ed393608f152726e8dd0e1ce1a0b9ed324b0ded2f5d37556adfa762b5d4c34faf776208e4e4875d8220e14a2719f407b029ec0904d6064df6f58c2b07776234a0270d2ce54912e28b9f2edfc2e214b23c12fc5a0b10e13d916eae36ae4ac362195f8ee28f391a05f69c5a48384a669be9c1399fa864090af4f301c9d8b72321606783f99157d2e719dab6023f4fb806bdc6314b0c713685885fd33b293fc0ae85ef08f985831239df655f682eb37df2c1a3e79ac57365d18585182a8a220b6ae8c48d06bb371a0a428d4f7e77c02d527b807e4022b67aeddfc003b8591d2c55bd8590fd86e044d4b2e71a3d79e95e76d8a2bb65c0b9203e5a213b39e94351489d655c589e5fc50284e1a23e86cd6a0268c43eb33431a6d16258c91d18e42f213715f9f5cc59a386ea5ccb490e05a1adb37aced8352554ad9eaed8f0160ed55d07b574c70e58eb739d0de41908126b226db0bbb8046e117e4226cca9a94d654f1929675f18f1f5f0b621d7fcfc45cfd8fc1c5ec6003241b04bb90caed5cf90e0a1957fe389f08ce7a359c773b1d985f9b9856664a0ebbe7bda411a349686b08bad5a20a2f41c15a3c50adf211a0514801c84b93a81c5736d580c35c9b97814d803dddfdd80f704ef47bc1006b56430a8db3a98b62669148f8ab7ef506c25c05d8aaaecb537cf803d82dc32945069ea49f4c9487ff975336c8edd0c1430041a847b5761344b4ac67b06162b9036b23e634f63a56fed7c50c253c5bbc68c3613c146bebfe289a235e49b63288095813c139b4889f2ecf5168e0ead9368a0290550cc052127c98437141d3c757dbb1e7035780c017fd58aa261f48c1cea9468725c0d242f2e314229c1912befaee704de9168add534d7ccff2e43a2666a8cd3e5290bd625d85f44c83fc0ee203a70d9f080e521bed93c38074ec210fffa21b6cf476b617fe265f71945aefc0cac2d25e6143439d6e1126fb53a9f782419c7ec0caa7a4a1b3cc2c4f16de8fdc4d617dd649a31b45273e5b3f055cf771ef0845308578649e1a0c3c00fb35b8a5b954fe46b5f3216bc0034449f87b4942fc2d605267e8ed5c4dd7f49eea561b8206cb8f177f402168efc76e4b20a6a8789e10eaf59b326c7c1542c8f47e968549aa76c7c12e22d8e1f7aba719bd046829836ecd5bee26f7648b838abec1d5397ec3427134a05ac9f5998a6e0a7144a41ee5009dca3af12c6a83b6755e7303d1a7c9a69e8fd5ec37d74ff21fb6a0d2b0c15a42ee1d10169c1f8200f4d26745888de07134b9b847d0c52789e6ca8a41593d1a6bcd1cf3d1df89bdea25e0f1c61038e351aa01ecd6ab6d4b316d858898a3671da2538f2d020d03fe65bf758654b074185b85d9b15f9c60f682b941eb8ab6de67b2586d2ca041d0171dfc9fcaf9945ac4f284541ada0b7c0c70be795db80309d0564a129eb2020980cbd6cc737239f914d6e001298dc0b8b719ed233f2456e90f1b1a1a3cae6630d6d99a01a4699cd19ce878bedfc9cbdb752200deeb6bf2c1cf96e7f76c6eba4aeb656927fd5fade7e45732544ccf7d7ef08b76c879b87adc7fc6ddae93404d6f8f5d90d75e48f9d294a945335b8b8eb32d5343ee52e4f7b78bb3afcd4ca87c974eb1f85da891c1970f177641b5e0f74629f3b30768242f862334b47b3c2fd388ab16d628137b021c633a26aeec49b5f45490e88b75040a95a1f48da9898572ec21113b5bca00e1287f8ad3820ca36df9260be5426a2af391b4c855ac0a5cd87a6f76c0f6ce984a92ffa0379e01e0c9b87c7bdbed4aea25b76b3f062144c5ccea9553bd32c30d8e5119d1648a553033d6a0591e8c0ea4d2024b8f4129b84244d01ad1c2d22eadac18c83b6cfdc81834baacb346593e2fe9b64d988404129eb2020980cbd6cc737239f914d6e001298dc0b8b719ed233f2456e90f1b1a4e80c49015f7e3372eaf67606d93d73289fbd618c6e7319201188cdb381a9e501135a0cfdc123fa256e09ebacf4dacc37d0aca48de196790c65d52226d3f30001aee03d498efa7f271afabdd9997e1c393bfa345a88525e6dc66bea95904c21618bd7d95e3e7bda1d0f318a99c06fb94a48419f04a97852d9cd91b62d72f758395c1b510d9fa63675bb731217e7e85af3d7bb57998e41af8302da49b194e6f0c1cf3d1df89bdea25e0f1c61038e351aa01ecd6ab6d4b316d858898a3671da253b2c89db3ba34e8568fac85a7b5d0de130f150696453c1110bbb259708695a5918a69d5409b5143cf60c95d216e0788540f6d779b64782323771117ecc555cc021d10169c1f8200f4d26745888de07134b9b847d0c52789e6ca8a41593d1a6bcdb56158bee904545dac89bfd39500df487954f30fb753ae3b88c2f1c6b593bd414fa1d86cc934ce0f30fd56cd777c61b20fad74ec9be9ba890212fbdf10b0f30720b6ae8c48d06bb371a0a428d4f7e77c02d527b807e4022b67aeddfc003b8591201de402bb48fdb1f108665fa925b899ecd509d6673f976d08cc57e6b2b47800e5fd798dc7a4505c1af1cef7be28a655026413fab9b1bd853df0d6d870c9e50b24c87f0e8b3f708353c643c1c09b9ac58bfe315f806f40bf8b2eb2e4ec3590834c410f3750c32311cce5e3cf21a344061bd4da12da671d0a6ff6d16f226bdd213630dceffd415eea759ff2d0529726b578d07e0bf1d3ee97c64300afdada04032c5950454cc20d515cda4651ee54756889d8d63b8a80f640402779a8ccf03ecc58d7ca013f2b683bc69ca1e5f0fd95bfc8ef0e2f8811d9b439a504f1919970f7914ed448b08751bc1eb47595d9beef33e323afdb441059bd2a41d2e268eb6c08308578649e1a0c3c00fb35b8a5b954fe46b5f3216bc0034449f87b4942fc2d60ec64d3d12b9fbc7bb43286c3547a19088df35ecd2becc7867bf153e24430778262b23b376fdebb9e26127efd7dfcba5e7abe83993ac77099d41acc1a76cb9700351489d655c589e5fc50284e1a23e86cd6a0268c43eb33431a6d16258c91d18e46917e096a35b46e3ab588fcd61b434b740375750894f91a6938ecd3d544d2e1aa63f43ef4763c7bf008f3d17121a109fc65c72d6f9eff9da959badbdf6962093e5290bd625d85f44c83fc0ee203a70d9f080e521bed93c38074ec210fffa21b7d816a10ee20b5da018b45d0055bee6d57120aee00e878ea42c65c6786268fd7059d94abdc736d4ac3dcba3aa0ff36832f1d45cd4420366234533826a9be440141a847b5761344b4ac67b06162b9036b23e634f63a56fed7c50c253c5bbc68c3a76a36e7136cd4fc3d08b13a3bce360a65fa15ce13ab98b1856146b922304b5494d5fac53e7c1dfd49d764f3bee6a772596fa02aa56e73ac565e4b750a019a0042f213715f9f5cc59a386ea5ccb490e05a1adb37aced8352554ad9eaed8f016042d56c041faa3a688c59232b8c87f63a12e8fa47470ebe019cc5f378a99f43fa1f3a5c421a3d8e3775c3867c81512daaf70c69e9bf4b025f21a8e761e2577701445f7d52a0405ebf12f1b69cf1e7637c6f04a7d67e1000df3357894e5bdb294dbcef1486dd01a28d5de378eed9265816e518b818a12c0c51cc9c1aa77c9f5d9018fe626744bf6ae873f7dd25d678e214edb780398545e2dbec9918fd8eaec70b445f86c96680f0a9a243a8d14e14f718ac5f801b713df816dc1a0b7481e2b37234ec6958ebe8a13efbd450ad2c7e77bb056051218bf13ee0dbb0776fad92b7a850c9ae85ea186cf0141ebae59d7487e9284e2493a95aaf8656fe618d7f0914034bf75538620f9ca48a473526c95e4fc0c68f2ab37e3cf3f86db4e70145b85b1879ef03a6a238859189ff5181ccd26b301001617c2d57a97a31756885f913ba8e814328f9d4fc3a0b3043a113be71f400f68da2685e81d081cda41e89e6a554044c34faf776208e4e4875d8220e14a2719f407b029ec0904d6064df6f58c2b077ff8447a2dc0f9721b0ec3eee5eec8e10178f2057f6b28290e152788acce38a98488fa8ff755aec4399d12e6cf7e29ec8458afbee614f960810056d4f546fc30f5267e8ed5c4dd7f49eea561b8206cb8f177f402168efc76e4b20a6a8789e10ea167fb18f1c171a5503db4e880225f2fbf00c03e853e4b2e7339bf02ca85adc008a62d0cd437b44ed4cc203b461c3788b0db55fa88a67fe44241258c0d7aee00b56b818ed393608f152726e8dd0e1ce1a0b9ed324b0ded2f5d37556adfa762b5d56d5f4f707a72324e8f8f813ab40627bed54c290ab4e2d8e1127f53f393c5cb6c0d2f994e0a5986f7614bf3466e9d84f11cb1408fb7b4b306b3272d2ead67e005bee26f7648b838abec1d5397ec3427134a05ac9f5998a6e0a7144a41ee5009d24088efe200b93fead958c0433cb6456f88838e50a66486aeef4ce7f5dbba154a4d4caef3d339b2841e2d9234ed3d0b84bc9544881bde2adacc84eb5d43e3f085c05d8aaaecb537cf803d82dc32945069ea49f4c9487ff975336c8edd0c143006a62597a175fbbece0c9deb22c91b0c9bf7fd2282d0d6915aa17e0eb5a1d8f919ef7eee826fb1c211eae54086ad212d7f4b93b243fcfe07fce1726ddb2d727056b08bad5a20a2f41c15a3c50adf211a0514801c84b93a81c5736d580c35c9b977e9473ab8a55565f5bad846a5f3627fa82bb66ca5eee7f7dbf9bc0a63b4bde0e3e4deb3ab6102433f4a435e8b8c7752fd590b4432939fcf053303b25109ba90c6d2ca041d0171dfc9fcaf9945ac4f284541ada0b7c0c70be795db80309d0564a5e88adf8ee98ddc6cfd3fe2b419d3ae2faa130d1489e875c399ee6d7b52fe0e8b72ef6bac5fa3f344ab69d6c239bdac26954ed055aaaad4537a3ba3779f2050772321606783f99157d2e719dab6023f4fb806bdc6314b0c713685885fd33b2932de3ac85a9e0531f4575a9201a6ad1a0f0376ff5061f95319a20e501ae42ebf67a91e5d352cf80307c53984739dbd7d605ea46c9d2b8b9a938dc6fbbd738e90d8725c0d242f2e314229c1912befaee704de9168add534d7ccff2e43a2666a8cd39dd7cb1dd7261f66f2d7ad0ae400b3b5dae9ed1a798e9d197cca290708cac373cbb6dbf7930fdf5ed7073270d63dbfcbc5a5335a931484c7e2e2325a3c741068f2d020d03fe65bf758654b074185b85d9b15f9c60f682b941eb8ab6de67b258efbbcd913ae5140eb3d61e1799ec6760c87601ce9fe5b257f87db1c4fd0299d54b9954186c8bceae8b5465ea39cd30b2cc6c50c4927b2c5c2122504653a6d00290550cc052127c98437141d3c757dbb1e7035780c017fd58aa261f48c1cea9469ca41970b15eff4f38aa898d530723e6bf5fccefd0812b1406cfb368bc33eabc83360dc6bc203eff445b56ab242640f6b8d271554bab9353838ed1045283e30d983bd8c44f9d09d1f17bfc97fc494f0100d054f50c927cf35fd71f0df7788a7fd8ad443fbebf93d26154f2a14991d80d3e359d924a1a97e65702d11f998747013357e41d5977cf2a2f61b3568fa836b90bbf84612e700ec518fd641479063a05a9898572ec21113b5bca00e1287f8ad3820ca36df9260be5426a2af391b4c8557c4426007c25a06a69d42f318ba0a63a817e0b867f36f8b21a49d43af8584da8b66e82f2221eb73c0eb275dcc1a3829e311cc4e9ef30bb7cafabb458ac353a00ae93404d6f8f5d90d75e48f9d294a945335b8b8eb32d5343ee52e4f7b78bb3af069974d163663a35f465fff0f79058e44cd297419186469b49b574d995c7c2f252c294a257d1dccf966add58dc0e87f60809faef453bd7c1d6a80410f943ab08c2252920ec60963094a935b6cfc483187d4092fa1d4af6c17a8fa93d90ffb826bd1a6c8b0323b6cca54d26016e294a4c6720ab0e20cd3287e9418029bff964f384eeae82c929e8dbfdc0a9a4c3f0226cae67af51acc6f462556498088a5ab50fc351f301f71458df98b139511ecd8a9e4e0d1b18471a4075ac6ae8c02443bb2efdda5459a65cab2bca11549eabeb6b255a704520c20456aadf36ebff586138feeae990deed91b03286811204b60777c301193959b938d8c24c0e1de703b1f30dca3af12c6a83b6755e7303d1a7c9a69e8fd5ec37d74ff21fb6a0d2b0c15a42ee40d189ec18c33e60049dd2a2f07652372747f3409c0ac9949985f3990dddcfffbba35f1356edb2e50cfe4ee13338db1f9a9a8f8f76b39d44128031650b362000cd4ca87c974eb1f85da891c1970f177641b5e0f74629f3b30768242f862334b45daacf426a87d73605cf4223b2cd0d1b2631fbe4ee275b4ce2f9ae581e68f5e6b1dbc5e3bbe2fca75fb8ad1cf3ffc142286819f6baee03a2c9f24787acf5c70cd2c55bd8590fd86e044d4b2e71a3d79e95e76d8a2bb65c0b9203e5a213b39e9440b5d75c14f381a6c4453b0899d618150803146ebfac832bbc22840957ff58da8aa1831862cef6b26f826da26f5f4a37ac4c41050d3907679ace9da6a0c11102d654f1929675f18f1f5f0b621d7fcfc45cfd8fc1c5ec6003241b04bb90caed5c05a0dc900f295f7b535f51b3194443a82cf05239958e805e5c8f7402e474f5a826f3802a1852c4e1236019e0fb6e33ecb39b8e3b55b88a302ba4d41953e8ff05e0c5b5ec1b83df6126744a6bff0cd76ab7c046997f1d7e0d6b4ce62133b3b35d8804e0a42df95e5d70862ea55bf521c0f239784d9614fc2cd381a53ba73ff037f7e0a417377ab381ea8b2eb65f9e54811e397c4b9c1655a171077af3a12a5d06e6cd4fa4fde4ab4313a607c588a97d4dcea63fd1265cee969d43148fcfcba89fa562c62d845c6b5262cb107f1d5b0d20be39bf393ad23e37dc8a7d64ae04eec56e4db6d44fe6812af97232b7780128ed497214b12c639bfe4ef45d984b5a5c07f19aa7b82162a73d6685224ea61bccacb80a0db0bc172b560edd9969bd06021e38364e81a4fcbb850214c7b00c553a2a17a1f9b08b6a1fa747e2fb13dcb7c52e0cddc7a21bd29918661b8aaccf85e157ee3d7979fc88ac3049fe39db8e034202f59b326c7c1542c8f47e968549aa76c7c12e22d8e1f7aba719bd046829836ecdd2467e75a2c8ca3c1c2e1d8dae4f460ec7616716ee966888b6fed7d9559be0f9cd92bf1f2397108e43f448df335d27deebd91f5b47070309c3a9af89f24d9b06f76c6eba4aeb656927fd5fade7e45732544ccf7d7ef08b76c879b87adc7fc6dd1781ea041a21f272d114b84e0a6a32786aa3a681daaf437f0d5399b31f62dadd6db073a0b7b73557ceb2f1484a3f89a4f10a3b8ea265a13408501add5d8f7002fb44a7e8d87632b76beeddf1cbee936661c8a537c5bc2d1e71605b18592e16f9be6df5b66f9992a11ac3a521d32ef75794e05fa547a72fa273f7d779dca4076d1e821c36a9d29f855d1e69daeebb786c3c7f43fdbeaf6191695512b3b8058900fc0ae85ef08f985831239df655f682eb37df2c1a3e79ac57365d18585182a8a29dba85feaabecda3c266b78e76f345d7954c99caf6db4aaca0c2e55d9b4b6ff5b9ed0e130314507510e8ed50152929f0a7617f8f9f0e40dac038b601601d9205d401fb05571ff76e3864698d47c66a25252b4dd6eba9f65ff8f5978b0359a2395b41ec0400005a000000000000000400000000000000")[..]).unwrap();
            let next_validator_set_inclusion_proof: dto::StorageInclusionProof = Decode::decode(&mut &hex_literal::hex!("80fd6e027f7a1bd8baa6406cea4d80d9327120fd2add6d1249bf1b6bfc3bdf510f5150eafc8680f4b114c88ad4d05bb6578e9587316fb125970fd1cd4c03df0c80decab11334eb3864a72812f2e02f4c49de4de19590bdf9101e8a1a6afa24eaa569988a7ab17cc6100bc67b7c55188db7e6928cdc8ca8ada3d30ff5d58c9cdc943f208957100642414245b501010f0000005104e12200000000121e24bfc0d5018cef5bd537a6a9db46802906fe32c0627444e277fbe3fcac7f72ae8c77aba0bed2014cbfb49c42b77a11c280ae202a1ac9f04c6472803cf70402623e1a34f82f0220dd80e0d6c3c2affbca4787a1b44117adf150768b2ae0060442414245092601f0380d397b8cb68efff0eb5735a1f6c326ba2ae1fc83968f6b53a520ca9a3cbc660100000000000000a410fb1d99d465cfc9c866d1a6bc22cdc373a65a5c6949c835bb9a44ebb8ef6401000000000000003cbebc8c3c4898d3790cf3a4242cca2a408c2fbb9b5755407df16576e181f759010000000000000076ed00fa2455aaf0b51f815976181801ebfdc49490f02a93a28e5c6fb6c3311301000000000000001e952f9872a336c528654ec9af4552145a236c94ec0392d81edad0938ffb15640100000000000000c2c2e732bdd09b1a8912a3116b366463bb868288b3ce6e1cbe3e49b4e7e0ea0501000000000000000a0ae4d2cbd40094292938464d13acfe5e01feb0dc780745ba8febaad6b8193c010000000000000090c6fc566399ca3d4266fc8e588e4db33e0ed4c59759804d9d2015926063de1e0100000000000000ce7b1a2246d249145c0531154b22474e2c671efb19e39a917b6a20d25d955d4e01000000000000001ed4a596e597e1542d82f7117f44d35c8cc5acf054b9329b53a348a0805847570100000000000000606a744931d02911d947ce9e48fe0a87423e713bb2b8631c7f98ebe9a3286c710100000000000000da8a50cc9745af5e7e55257bbf9f1b0ef4bd08ec5c4b1b8d38106fd32f3cae0401000000000000008aa80106b9bf9543726996d571da80d31515b8b0e4e1433bba24541fd5803c2201000000000000000c19fbbf9dfee27e90421556427ecfb533147efee7cdcfd63a5e5605f35c7a7401000000000000001c9a8bb6b424510eec51389ff938cab6ab9bc6af0eaf400daeae09034cc7e0180100000000000000347bc0b3971351b1c4602ba043db7247f5e47b36be22e78fdb3215ba74671d4c0100000000000000b02da965e9786cec099601a3c41f81c26272b55bf3a05b2106ef96ca962c932a0100000000000000a0d6165a41f48d7eca14d4bdfcbee8b65b57d948494b39cab19715ece5e6ab5e01000000000000003cc7a84ca415a3188ae375976a560a4b2e84946aef2a5714e428304f01d59d620100000000000000ee0b3601ae2499ae47df26ad165e97137c05f45432538b38babe76b04232921e0100000000000000ba0fe9f2282505e8ba44e8376fbbedcd94226b8668dd7d4944e4ffa7835b760e010000000000000058087ea312fce332c695b1a0ecaa6ff92a434b0544032373ff76681a73c82d3b0100000000000000f67d0cd0496f4e6b63d5cf44b2d81aa435b2b301b2518904b10da490d0521d0e01000000000000009ed3c115a1ae4bb5deb89e4704fd5257354f9ade790ef964953a61c0a7bce3120100000000000000c2a43f0e3ffe17327b0ceebd7a5271c43d7196be2b5e5231c484739b5c30ea6f0100000000000000f8aee915f4fd2619c24438145982b9b80b4d1ef21caea4d78850b928802bbf720100000000000000863f9af1c8e857a131f9c1b6e4081eea8a2bd3479344d900176ac7fe00fdd54901000000000000006acddf5d0bfafa9192129394931bdbd8ec1eb7c9789130b9da7a44b2d8c17107010000000000000022919327b2980b02f874ace62b3667dc84250aeb6ad17e0532ecf20f7a893e130100000000000000ecb83703aef5fd75d176ff7ae5cb2b0b8cb7c6d09c689266712c51c87db1784801000000000000004a6d1bb5dfba6e7a40e6e6a1410c6cb849c6ec955183eed7360d1324eaab58330100000000000000f209388de0a83c211e88f99451b4e72443e2b1a43ca5c2a19e8b20b938c337250100000000000000983d61337fc5ff941ea2522981d01ae4e652b78cea8606e8b99a044af604013f0100000000000000a2a3d6d5de5b0eeef375d1875c825b2c9cc15850b45da1c5987b9a181af2630e01000000000000002650e2fe61dc584911552338cc38e01e94181ed7491ce5b4d80ec12404bce93401000000000000002a1f7669434fe9c2d64c4a75ff25979e2536828323b6df0b1dd36a5ddb98e84e010000000000000020ec9497154712bf4fe5b2c65a1082685d8dcfaab2fc52239e5064eec366d7130100000000000000b6610d6baa2a5cdc582ca6a59e04b81cb4b8217171a8ec65953e66c28b49af35010000000000000090a09621a611cf403f401e61606da4fd7e4da75c844e787c3677699f39d9ba3b0100000000000000808178752fb5bc2d7ad6d9606892baa9a6cdadf1cbd8734228b4dc8517540712010000000000000098041eebd9dac1433a47655e2e8b8a1b09f2f901182ddd9a506c398ae9a0c47c0100000000000000d6cabdd9eedfdacd87aa65e7e45c709d2e1272317309128e7fd01de2f1dbf1150100000000000000faadef82c6a2822860afd1e1a49fef73b407d7bc67518086bf7e0eba12617c7e010000000000000040fd58bf5c39ae92339afc37ab17980000a33cb66747e3be472541a63c0a5c7b01000000000000008268f51c55c77d48f901d48fcb01bf2e7ff1d70ced125fbf17d4313f38c1866a0100000000000000bed56a1855b837e61b13f2bcba90e348c10759433aec6a9769eb44124ae60f6401000000000000002ceb89a06fd64ef2af7d784df5df6b5a48ceb73fa9030f58bc8167829bbc67460100000000000000b0cca2b4bca85b530b70f03df31336e5993bccc5affc7dd262bbd72fb68971020100000000000000049355c8e5ec54f0f96cf239a233ef1a0706153ee6fdb12d48009cc3f5940b630100000000000000284d01879de483c1eae463426c64c0dc0a7e6c420b7c0c58f27a6659a85bce5e0100000000000000aaefccbf988f905a828ed6b7ca125a55daec4fc51d4e3cd2532e02b2fabab47d010000000000000072750851187e1e94d2c928ec2bd37ecf7bc1f768e69a8ea4577cca8faa7d307d0100000000000000ee5e9c468922fdf80c0ced4ff9a1f17381e69d85cd3c1a70a4fe5f0aa3b4ac0b010000000000000004ee1ebf7dbfc0aa5df51c7e1d7accef6131dc55f0e0596b8cae876eed3570760100000000000000ba28c28e2664419ede221d34c075b0746881aa3bc2d308d5f569e0f832c6d2550100000000000000005428217cd90c39504c2ac004637d57983033be807f0c4f1d7e13384502f061010000000000000076f47ee5f017237c74a306d586011a966791a9338adb5e002058e221a1eecf060100000000000000dcb1dc506e297fc76d09f02e111861af2bc031427176c241003e6c127b4bb2640100000000000000480d37472d32b8d7ae17c7ff220c502e68a28296b8730fcf93dfa28f2201115301000000000000002a6d729043962c310c5b121c7cab5b8f2051149bc804ba38ab2fffae71949f280100000000000000e498815592465d833a5f5d0dc93bbc1ca2863ec7089f137317575685ba0e010d0446524e4b992501f0445f7d52a0405ebf12f1b69cf1e7637c6f04a7d67e1000df3357894e5bdb294d0100000000000000c2252920ec60963094a935b6cfc483187d4092fa1d4af6c17a8fa93d90ffb8260100000000000000c351f301f71458df98b139511ecd8a9e4e0d1b18471a4075ac6ae8c02443bb2e0100000000000000983bd8c44f9d09d1f17bfc97fc494f0100d054f50c927cf35fd71f0df7788a7f0100000000000000e0c5b5ec1b83df6126744a6bff0cd76ab7c046997f1d7e0d6b4ce62133b3b35d0100000000000000b5af6489c3676b81365bd07bbc40c41833c8dca8cf433287805fcd3bbb0d35ee0100000000000000e6cd4fa4fde4ab4313a607c588a97d4dcea63fd1265cee969d43148fcfcba89f01000000000000001aee03d498efa7f271afabdd9997e1c393bfa345a88525e6dc66bea95904c21601000000000000004f5f8c8ecb586670a7935c4395eb85d3072311f5b1942e6bc2d658e7b835dc1501000000000000004bf75538620f9ca48a473526c95e4fc0c68f2ab37e3cf3f86db4e70145b85b180100000000000000f19aa7b82162a73d6685224ea61bccacb80a0db0bc172b560edd9969bd06021e01000000000000000a5cd87a6f76c0f6ce984a92ffa0379e01e0c9b87c7bdbed4aea25b76b3f0621010000000000000076d3a063b9d141719b87c9dae10a52a94b5f8818d1bd12f41308a7654bb890450100000000000000c56ba044a5f3e9e4541b1de73b75db78c22838eca8a7bffe4f9507dcfdae357801000000000000003ff89a98ee0654affbf2fd1c97d9cb4aab58e96855ec13bea0a88c06ce4ac05f01000000000000008e6c6bac12f96d9b8ac1e021e028ad0d20a0056fb4c0df6b8e0c45ef53779fe101000000000000002c5950454cc20d515cda4651ee54756889d8d63b8a80f640402779a8ccf03ecc010000000000000006fcca0083a3bd75816f7a3a648d60a93188802bafa3d11653d66a9dae47c57f0100000000000000445f86c96680f0a9a243a8d14e14f718ac5f801b713df816dc1a0b7481e2b3720100000000000000fb44a7e8d87632b76beeddf1cbee936661c8a537c5bc2d1e71605b18592e16f9010000000000000024c87f0e8b3f708353c643c1c09b9ac58bfe315f806f40bf8b2eb2e4ec359083010000000000000056b818ed393608f152726e8dd0e1ce1a0b9ed324b0ded2f5d37556adfa762b5d01000000000000004c34faf776208e4e4875d8220e14a2719f407b029ec0904d6064df6f58c2b077010000000000000076234a0270d2ce54912e28b9f2edfc2e214b23c12fc5a0b10e13d916eae36ae40100000000000000ac362195f8ee28f391a05f69c5a48384a669be9c1399fa864090af4f301c9d8b010000000000000072321606783f99157d2e719dab6023f4fb806bdc6314b0c713685885fd33b2930100000000000000fc0ae85ef08f985831239df655f682eb37df2c1a3e79ac57365d18585182a8a2010000000000000020b6ae8c48d06bb371a0a428d4f7e77c02d527b807e4022b67aeddfc003b85910100000000000000d2c55bd8590fd86e044d4b2e71a3d79e95e76d8a2bb65c0b9203e5a213b39e940100000000000000351489d655c589e5fc50284e1a23e86cd6a0268c43eb33431a6d16258c91d18e010000000000000042f213715f9f5cc59a386ea5ccb490e05a1adb37aced8352554ad9eaed8f01600100000000000000ed55d07b574c70e58eb739d0de41908126b226db0bbb8046e117e4226cca9a940100000000000000d654f1929675f18f1f5f0b621d7fcfc45cfd8fc1c5ec6003241b04bb90caed5c0100000000000000f90e0a1957fe389f08ce7a359c773b1d985f9b9856664a0ebbe7bda411a3496801000000000000006b08bad5a20a2f41c15a3c50adf211a0514801c84b93a81c5736d580c35c9b970100000000000000814d803dddfdd80f704ef47bc1006b56430a8db3a98b62669148f8ab7ef506c201000000000000005c05d8aaaecb537cf803d82dc32945069ea49f4c9487ff975336c8edd0c14300010000000000000041a847b5761344b4ac67b06162b9036b23e634f63a56fed7c50c253c5bbc68c30100000000000000613c146bebfe289a235e49b63288095813c139b4889f2ecf5168e0ead9368a02010000000000000090550cc052127c98437141d3c757dbb1e7035780c017fd58aa261f48c1cea94601000000000000008725c0d242f2e314229c1912befaee704de9168add534d7ccff2e43a2666a8cd01000000000000003e5290bd625d85f44c83fc0ee203a70d9f080e521bed93c38074ec210fffa21b01000000000000006cf476b617fe265f71945aefc0cac2d25e6143439d6e1126fb53a9f782419c7e0100000000000000c0caa7a4a1b3cc2c4f16de8fdc4d617dd649a31b45273e5b3f055cf771ef08450100000000000000308578649e1a0c3c00fb35b8a5b954fe46b5f3216bc0034449f87b4942fc2d6001000000000000005267e8ed5c4dd7f49eea561b8206cb8f177f402168efc76e4b20a6a8789e10ea0100000000000000f59b326c7c1542c8f47e968549aa76c7c12e22d8e1f7aba719bd046829836ecd01000000000000005bee26f7648b838abec1d5397ec3427134a05ac9f5998a6e0a7144a41ee5009d0100000000000000ca3af12c6a83b6755e7303d1a7c9a69e8fd5ec37d74ff21fb6a0d2b0c15a42ee01000000000000001d10169c1f8200f4d26745888de07134b9b847d0c52789e6ca8a41593d1a6bcd01000000000000001cf3d1df89bdea25e0f1c61038e351aa01ecd6ab6d4b316d858898a3671da25301000000000000008f2d020d03fe65bf758654b074185b85d9b15f9c60f682b941eb8ab6de67b25801000000000000006d2ca041d0171dfc9fcaf9945ac4f284541ada0b7c0c70be795db80309d0564a0100000000000000129eb2020980cbd6cc737239f914d6e001298dc0b8b719ed233f2456e90f1b1a01000000000000001a3cae6630d6d99a01a4699cd19ce878bedfc9cbdb752200deeb6bf2c1cf96e70100000000000000f76c6eba4aeb656927fd5fade7e45732544ccf7d7ef08b76c879b87adc7fc6dd0100000000000000ae93404d6f8f5d90d75e48f9d294a945335b8b8eb32d5343ee52e4f7b78bb3af0100000000000000cd4ca87c974eb1f85da891c1970f177641b5e0f74629f3b30768242f862334b401000000000000007b3c2fd388ab16d628137b021c633a26aeec49b5f45490e88b75040a95a1f48d0100000000000000a9898572ec21113b5bca00e1287f8ad3820ca36df9260be5426a2af391b4c855010000000000000000000000054241424501010a788792c89154bafa0e642a0c4143947c6d62e0b51031bfdb74d2a5f31e474ddbd6396a8dab741aee910286871628344ea6d3a5349f2dca30e82cc1dd3b26880ca9029e6e027f7a1bd8baa6406cea4d80d932b0a080b8e055dec166f62102fde0cec5b354ce9ed63f268552fdb40be6a3094ff542c54c5f01d47f736dfc5807b7a3033e859a8dfd0400803837f9e673ee9fddeac3c3725bf6a47ee972f8a6a02ece883bd3c935152986d680d16dad8d5b79c031830ae4c85dd12ffe63923aa58a2494ad269b85a5a5d331aa8067485462d5200506f662399fb390026a27505f90d08ecd2915aa2ce5966fc8a407990180016080b5d050679014205cf0e8d71f6f7bef19aac026692e9b0244b590e114072344b0807f0ee35613089581a06ddd04a3ee2ecaa0b66e4700383d7c87d5fbc182cdcf8280fbb0fbeef4e03769226c714de9ec6e3a6e011c24eb44cea81c31ea3e68b3b5390dc10680bff58039f6069ecb5d3e0d0eceb324c65ff316659261b1a200d1e398f79c70d5ed69308016862070dd94c235b6bd8bdf9e36a0802896f9326023a3b2f1a800dea4323926800c6c92db3bb508b97cb72218f499588dc3d88bd40302e64bb4378748ada8b51e8088bdb0cac5a5f2104187c6265fd56fe918e9d01f1419d81c634b8edc310d4ca68093b4202075803222e5bd965576d23faf002c576dd00a17491ec27a6a2c511a7e806cd1d130790a7c2591974a05a56242c1b95640c00f3af9ac6561212bd462975c8068264e6e29190379643fbb028141335583bd61465883da1aecd0d6ec638539e2803554e8c619c26e2bb67aafdce9d55552e464378fb77745b63390af3af6db79778004eeedf1d3c9dec83c629a44c467fee3a8a7dba7200b5aaac906a6da25db3db6809cd475d34983dfd32d649a22057e71341be8e25e82b34a628d044349d5a10ac980d54d6bacbfddc26d090dcd0988ceb949e2d9b92eb00260e77f827c19f7dd9ba880997889269ae3c8db7d2acd23aeb269ea9964f1e03d0fb2b6545094aeeb898c4580188777c840826a58165983f2f4ff92e28a10eb21b63cfbdb6cf563c470cbcb000fc85f0120fd2add6d1249bf1b6bfc3bdf510f809c1b1c4b1cee0aa024fcb364baab4e3274cd40e69fcb7bcd2062ab062af41796809c1b1c4b1cee0aa024fcb364baab4e3274cd40e69fcb7bcd2062ab062af41796")[..]).unwrap();

            let next_validator_set_storage_data = next_validator_set_inclusion_proof.stored_data.clone();
            let next_validator_set_inclusion_proof =
                parse_rpc_inclusion_proof(next_validator_set_inclusion_proof);

            log::trace!("Start to prove genesis");

            let now = Instant::now();

            let proof = prover::proving::prove_genesis(
                parse_rpc_block_finality_proof(current_epoch_block_finality, Some(thread_count)),
                genesis_config,
                next_validator_set_inclusion_proof,
                next_validator_set_storage_data,
            );

            log::info!("Genesis prove time: {}ms", now.elapsed().as_millis());
            log::info!("===================================================");

            proof
        };

        let previous_proof = {
            // genesis block = 5f016dcad02701fe71d65212bfa81ea47cc360d4948197a8b383b429ab8ef056
            let current_epoch_block_finality = Decode::decode(&mut &hex_literal::hex!("f0445f7d52a0405ebf12f1b69cf1e7637c6f04a7d67e1000df3357894e5bdb294dc2252920ec60963094a935b6cfc483187d4092fa1d4af6c17a8fa93d90ffb826c351f301f71458df98b139511ecd8a9e4e0d1b18471a4075ac6ae8c02443bb2e983bd8c44f9d09d1f17bfc97fc494f0100d054f50c927cf35fd71f0df7788a7fe0c5b5ec1b83df6126744a6bff0cd76ab7c046997f1d7e0d6b4ce62133b3b35db5af6489c3676b81365bd07bbc40c41833c8dca8cf433287805fcd3bbb0d35eee6cd4fa4fde4ab4313a607c588a97d4dcea63fd1265cee969d43148fcfcba89f1aee03d498efa7f271afabdd9997e1c393bfa345a88525e6dc66bea95904c2164f5f8c8ecb586670a7935c4395eb85d3072311f5b1942e6bc2d658e7b835dc154bf75538620f9ca48a473526c95e4fc0c68f2ab37e3cf3f86db4e70145b85b18f19aa7b82162a73d6685224ea61bccacb80a0db0bc172b560edd9969bd06021e0a5cd87a6f76c0f6ce984a92ffa0379e01e0c9b87c7bdbed4aea25b76b3f062176d3a063b9d141719b87c9dae10a52a94b5f8818d1bd12f41308a7654bb89045c56ba044a5f3e9e4541b1de73b75db78c22838eca8a7bffe4f9507dcfdae35783ff89a98ee0654affbf2fd1c97d9cb4aab58e96855ec13bea0a88c06ce4ac05f8e6c6bac12f96d9b8ac1e021e028ad0d20a0056fb4c0df6b8e0c45ef53779fe12c5950454cc20d515cda4651ee54756889d8d63b8a80f640402779a8ccf03ecc06fcca0083a3bd75816f7a3a648d60a93188802bafa3d11653d66a9dae47c57f445f86c96680f0a9a243a8d14e14f718ac5f801b713df816dc1a0b7481e2b372fb44a7e8d87632b76beeddf1cbee936661c8a537c5bc2d1e71605b18592e16f924c87f0e8b3f708353c643c1c09b9ac58bfe315f806f40bf8b2eb2e4ec35908356b818ed393608f152726e8dd0e1ce1a0b9ed324b0ded2f5d37556adfa762b5d4c34faf776208e4e4875d8220e14a2719f407b029ec0904d6064df6f58c2b07776234a0270d2ce54912e28b9f2edfc2e214b23c12fc5a0b10e13d916eae36ae4ac362195f8ee28f391a05f69c5a48384a669be9c1399fa864090af4f301c9d8b72321606783f99157d2e719dab6023f4fb806bdc6314b0c713685885fd33b293fc0ae85ef08f985831239df655f682eb37df2c1a3e79ac57365d18585182a8a220b6ae8c48d06bb371a0a428d4f7e77c02d527b807e4022b67aeddfc003b8591d2c55bd8590fd86e044d4b2e71a3d79e95e76d8a2bb65c0b9203e5a213b39e94351489d655c589e5fc50284e1a23e86cd6a0268c43eb33431a6d16258c91d18e42f213715f9f5cc59a386ea5ccb490e05a1adb37aced8352554ad9eaed8f0160ed55d07b574c70e58eb739d0de41908126b226db0bbb8046e117e4226cca9a94d654f1929675f18f1f5f0b621d7fcfc45cfd8fc1c5ec6003241b04bb90caed5cf90e0a1957fe389f08ce7a359c773b1d985f9b9856664a0ebbe7bda411a349686b08bad5a20a2f41c15a3c50adf211a0514801c84b93a81c5736d580c35c9b97814d803dddfdd80f704ef47bc1006b56430a8db3a98b62669148f8ab7ef506c25c05d8aaaecb537cf803d82dc32945069ea49f4c9487ff975336c8edd0c1430041a847b5761344b4ac67b06162b9036b23e634f63a56fed7c50c253c5bbc68c3613c146bebfe289a235e49b63288095813c139b4889f2ecf5168e0ead9368a0290550cc052127c98437141d3c757dbb1e7035780c017fd58aa261f48c1cea9468725c0d242f2e314229c1912befaee704de9168add534d7ccff2e43a2666a8cd3e5290bd625d85f44c83fc0ee203a70d9f080e521bed93c38074ec210fffa21b6cf476b617fe265f71945aefc0cac2d25e6143439d6e1126fb53a9f782419c7ec0caa7a4a1b3cc2c4f16de8fdc4d617dd649a31b45273e5b3f055cf771ef0845308578649e1a0c3c00fb35b8a5b954fe46b5f3216bc0034449f87b4942fc2d605267e8ed5c4dd7f49eea561b8206cb8f177f402168efc76e4b20a6a8789e10eaf59b326c7c1542c8f47e968549aa76c7c12e22d8e1f7aba719bd046829836ecd5bee26f7648b838abec1d5397ec3427134a05ac9f5998a6e0a7144a41ee5009dca3af12c6a83b6755e7303d1a7c9a69e8fd5ec37d74ff21fb6a0d2b0c15a42ee1d10169c1f8200f4d26745888de07134b9b847d0c52789e6ca8a41593d1a6bcd1cf3d1df89bdea25e0f1c61038e351aa01ecd6ab6d4b316d858898a3671da2538f2d020d03fe65bf758654b074185b85d9b15f9c60f682b941eb8ab6de67b2586d2ca041d0171dfc9fcaf9945ac4f284541ada0b7c0c70be795db80309d0564a129eb2020980cbd6cc737239f914d6e001298dc0b8b719ed233f2456e90f1b1a1a3cae6630d6d99a01a4699cd19ce878bedfc9cbdb752200deeb6bf2c1cf96e7f76c6eba4aeb656927fd5fade7e45732544ccf7d7ef08b76c879b87adc7fc6ddae93404d6f8f5d90d75e48f9d294a945335b8b8eb32d5343ee52e4f7b78bb3afcd4ca87c974eb1f85da891c1970f177641b5e0f74629f3b30768242f862334b47b3c2fd388ab16d628137b021c633a26aeec49b5f45490e88b75040a95a1f48da9898572ec21113b5bca00e1287f8ad3820ca36df9260be5426a2af391b4c855a806fcca0083a3bd75816f7a3a648d60a93188802bafa3d11653d66a9dae47c57f6db971e0df82c1fc09a0de7ea8f17a445d355e02b93524735578b90c35cd14fa6ecf562f88366378b06671b11cbe069d8a7b3776348aa749d12584b66d0b450a0a5cd87a6f76c0f6ce984a92ffa0379e01e0c9b87c7bdbed4aea25b76b3f062137a823db73046d7d0f851109821d7c158f78e55c6b0e5fe688d7079859d86c6a83b41ff5facf9d62354a18d228f26525e66925b5019708dff40738367cc762081a3cae6630d6d99a01a4699cd19ce878bedfc9cbdb752200deeb6bf2c1cf96e727d624c70b3b285efec1faeae94ad10741cb6be9505e35a3835b1d96f6ea1962131100e4143a9be8c195a72194ed449ffcf490d71a436fe1d123f5d987617c0a1aee03d498efa7f271afabdd9997e1c393bfa345a88525e6dc66bea95904c2162b7ffcd17f7a6810dd94b06c5cb5909ad62778798e8f77c8292b66d69e512a2dab3aefb0ee1ff844c0a3ae522522afc9fd2431423e069047b694d3f55c7a89081cf3d1df89bdea25e0f1c61038e351aa01ecd6ab6d4b316d858898a3671da253f1bb89152fa06a90573e6c62203280b697669b467cfc3213bcc4d1e148c4073eb1d19b40444285349d7a60d2fb2ee32d7a43505aa956418309a935a7d7ded2051d10169c1f8200f4d26745888de07134b9b847d0c52789e6ca8a41593d1a6bcdca60eaee0ee78546e16041a663411c16b74a83246c6a3afee757fe35881d1d17b69e59079b9a42e394d3c5d6b17b82555c0027c69c2fbafea7e0ca8e3924f80d20b6ae8c48d06bb371a0a428d4f7e77c02d527b807e4022b67aeddfc003b859157815996a83f85523c937822908c5a7d8c2ebacca24e589de893cf746633f743e7d7adb8301fa3361d139de424cdaeadd8b1ab3cf627f77aafdd54a6b087eb0924c87f0e8b3f708353c643c1c09b9ac58bfe315f806f40bf8b2eb2e4ec359083004fe4d552e97c5d5a63a6eb3890e27b79f431faed12100827ab1cd9960783090407914a76c4c48c9cb4aa697d7b6b7714a6052580e456e855a8962f90383c0d2c5950454cc20d515cda4651ee54756889d8d63b8a80f640402779a8ccf03ecca5714327844b6a47e15c701802a4baace87a7fa7216e426d33b58d7cb75645c4b43e6ab6c41d9babad3bf1c7feb143817ba9a6d209a515ae0a9471d14df9ed00351489d655c589e5fc50284e1a23e86cd6a0268c43eb33431a6d16258c91d18eba121f4f0d363600b64801bb211d6d5c249f05ce9222e048586fb2d675bae1f7a76d3cd8d9369a4daf2dd7a92da5847cce993610adfe104d8ea73c0856a3300b3ff89a98ee0654affbf2fd1c97d9cb4aab58e96855ec13bea0a88c06ce4ac05fc4b3c58a916265d123849cbf514111f3c6205910cd8699e0c356204bdbf79d57000ebe96b11de0864852b21629e1db4db7cf11ea137db6826ae27d7238151b0f445f7d52a0405ebf12f1b69cf1e7637c6f04a7d67e1000df3357894e5bdb294dcb19c0f9457597ec74911c879a80d377f9096a5ba9f1f2edf9863bb16a6a1fe71f7bbb64642f19ff4d6acf082591424c76e7c46c7825e8fffc5b6c1c6a33a00c445f86c96680f0a9a243a8d14e14f718ac5f801b713df816dc1a0b7481e2b372c3d70de58c17edcbfec9e5db93a32379b65a2fad3d899a41d6b66a9818910831b07f7a706eb90054a93333aa611aacd31ac066cc098e4fc34a9743c9f19223034bf75538620f9ca48a473526c95e4fc0c68f2ab37e3cf3f86db4e70145b85b18c0a3ab6958ec785288ee063d10d80d971dfdc39fe689d08b60b131be58580951f13cb295f4f3a7359fef968f9749c0da5df724e423570cb9499282460c73400d4c34faf776208e4e4875d8220e14a2719f407b029ec0904d6064df6f58c2b07712b9252fe028a3ecdd4b8ff57730c7ed079e516dbcaf323cd2b2be9bff5e02ce7c94f22f8070e4663b49b60fc102cfa3f70d596fe38ed7276abd3a5e5a59e4034f5f8c8ecb586670a7935c4395eb85d3072311f5b1942e6bc2d658e7b835dc1553af48ef01ebe8f9047296b4eea3fa12996175010297588b79d56f5c121fe5e1376ae65da50d0c8f284a1fe2b5365166ee6467348d638196d00fd52e5d0710005267e8ed5c4dd7f49eea561b8206cb8f177f402168efc76e4b20a6a8789e10ea06ea6f26740333a412c2e5a2390d634b06c60101779bba945e1084f6aea80e4c2926cd0cdd68a046edc96e65795f74e4528bf95e808f96ea2e704742ec67db095bee26f7648b838abec1d5397ec3427134a05ac9f5998a6e0a7144a41ee5009d95fd775879d5ae28d326ee5a4a4cc09c10055a2667ed7f09db7ee4c9d2abf7236eaa377ed92471d489678817b90898348bd54c074e9f57e9a6147a6bd6266a066b08bad5a20a2f41c15a3c50adf211a0514801c84b93a81c5736d580c35c9b976ffdf0582594765928bb477e05bd6afc2844b5791038c540bb92e4867d41550aa9fe4d79f4828981b84757f3df34e35548a697a3b27e8673f9e00e37cfc6a40d6d2ca041d0171dfc9fcaf9945ac4f284541ada0b7c0c70be795db80309d0564a45605aa70a853da2717fce9e6fdf0e552ce12dbcb67a0a8f3504ede9246e39b19947c16e6555e37f0220389e0b3ec58be0cc7db8aa7fcc48ebe022195652a90e76234a0270d2ce54912e28b9f2edfc2e214b23c12fc5a0b10e13d916eae36ae423bdfdd4194b3992d53da4b3871c84324b1fe00be87864de60786a8bdd5b8b399c92d7b1203b84d852c31dfcf9155ff0df892322c580909c8612dd6ea1ed180576d3a063b9d141719b87c9dae10a52a94b5f8818d1bd12f41308a7654bb89045fc652dd165f4bd1a47a2d4ae4eb23823e144501eff5729a12d6a97f5bdb26517e9e115f8700f5a58211a899ef174e9c8c4a241e8760742fa9117e81948cb86058e6c6bac12f96d9b8ac1e021e028ad0d20a0056fb4c0df6b8e0c45ef53779fe1b63aa95ac729a120410bbee053efe87b845f792beefec5a5720b82850bace31735ea87f3deceabaec6833e5464e9985e99b70e033d04ca5e3a9d7dd0635598028f2d020d03fe65bf758654b074185b85d9b15f9c60f682b941eb8ab6de67b2580b88be7701bd8b649196b16416026f883c7a12493ee5611a488f29060687a8b82cdc1f941a37a56a9dcbafbe81870be273ce4762793909efb24ce1d1a213c80890550cc052127c98437141d3c757dbb1e7035780c017fd58aa261f48c1cea9463b07aa0f5a08b33f3b7b3962bcf9b08bd86e4ae39ed7f8657ae1d9ab82b9bbc831a3667ebe99e73e080fef511ba055c70d52c648fffb34ae357459ec322c8b0a983bd8c44f9d09d1f17bfc97fc494f0100d054f50c927cf35fd71f0df7788a7fcbb85eec0c5710b50c41bf77bb40e516d2653660ac99962f154d121a974d704886851c1db4dbe9ec547536b470c1926d52f57a2c1307fa63aed938b4b3b92703a9898572ec21113b5bca00e1287f8ad3820ca36df9260be5426a2af391b4c8551de516447192956794cd6ff04e6ed6d9913304939122c9a94412340cf065e492a483bff58de688e9d63f01542f8fdce994eb7a228567e6d60789a69e5ecddc00c2252920ec60963094a935b6cfc483187d4092fa1d4af6c17a8fa93d90ffb8260f6a7e1ce16b897e70fe1f1368cc357698e1337c790bc4efc117710babf9ff87b08224e845e6e5658f201f770a21bcc3acf592f87c28a509aa94d29dee176f09c351f301f71458df98b139511ecd8a9e4e0d1b18471a4075ac6ae8c02443bb2e522c7ca9e2cd1194a3f5d2591d850fd04287d0f6bae051bf0ba8d6b5316e18b1d016f76fb549c193fb591a75f8968848ee2daacc8e83af47cea1b4e677872006c56ba044a5f3e9e4541b1de73b75db78c22838eca8a7bffe4f9507dcfdae3578c0bb18df5040c116d8996a8ab55615c933e8dad2ebc1e07347471e6d9930442dbf36e3111fad0b07395d9303d76f73ccd65212ebfd5c80608eb47f24aab85602ca3af12c6a83b6755e7303d1a7c9a69e8fd5ec37d74ff21fb6a0d2b0c15a42ee8026202d3970c34f768ac7a79f0a132a82eb01af3579dabad53eea335681576c02440308db4bd90f2db5fa8865b295e5f1eafa87f677f1d549a9194cbc91b605d2c55bd8590fd86e044d4b2e71a3d79e95e76d8a2bb65c0b9203e5a213b39e9415fc57e12a2d2698cda5e6543bbeb41fc06fba2dbcfbe5d98cb84c28d6b285700d93f47eec397b1d845b88c1dff0364b2427e4094d92e1150b4479bab6cc5b04d654f1929675f18f1f5f0b621d7fcfc45cfd8fc1c5ec6003241b04bb90caed5cb59a477b908a804fe617eeee586a4a9ab4d48e8238fc6e9858ed947dec8aa812af781b2325c9071f221f95ecddc396851329a7f5d5d365e23c2150cd82a1840ee0c5b5ec1b83df6126744a6bff0cd76ab7c046997f1d7e0d6b4ce62133b3b35dcea29918bd0b6ff8479d32f042fca28bc987ea6af693303514ebae2756665f327e6521d3ac4064abbeae78874ad47d77f1fce2493d534b8e4fbb081233261905e6cd4fa4fde4ab4313a607c588a97d4dcea63fd1265cee969d43148fcfcba89f95f4ef7d0095bd25d3f47088acb75d67bbed561d1e81cc1dc835f6ea3b13e6f282015b43bb83753d5aa82822dda5d92dbac021525fdb441e9820b208c95e7b03ed55d07b574c70e58eb739d0de41908126b226db0bbb8046e117e4226cca9a947ad9534468e0e3087829a2695d571488cf30548eac18d715033b7788308b6ab59fa9fa9f84955200039bca4cad3c0963ddbe21748394557509d5cebebaeaaf0cf19aa7b82162a73d6685224ea61bccacb80a0db0bc172b560edd9969bd06021e1b6b7b3c11f1b0f9c314efb2297f673b5133749ffc0a0a75e04fedfef1378be7c33ccb43b2fe6cb68b03805c128112a014c612a1504a45a1ecefab655a810309f59b326c7c1542c8f47e968549aa76c7c12e22d8e1f7aba719bd046829836ecdd8a7fb9d8e74376232a6a02e877ec83790ca878abb7e20ca321be25fed36457bc0e04335d34395620b5a4157a2fc1a72464780f2837d986b3b223245551cb106f76c6eba4aeb656927fd5fade7e45732544ccf7d7ef08b76c879b87adc7fc6dd80bc633c47dfa3e76bd9366d2274dd6141fa55f67496d0e036482b315597e01555a01c5fa50909c618fca993d4cd3c84e612cdb02f5f395b692de9a7baaad70cf90e0a1957fe389f08ce7a359c773b1d985f9b9856664a0ebbe7bda411a34968f894236cbdf3338410344cfdd0ee792de6ae0675b6bea06fcfa6aa5ace119f42963b0b18b28a2272f842db10be101d08dd3a16bc47bda3d0d20f801faf987e05fb44a7e8d87632b76beeddf1cbee936661c8a537c5bc2d1e71605b18592e16f965757417c3d097db54ecb2c66f8238c7c3f4bd75996da201bac0da297e9bc9b559deecbb55414198830d63f4ee2dcd2bc5b9d2ee9856630fa5dc3c97ae868d0bfc0ae85ef08f985831239df655f682eb37df2c1a3e79ac57365d18585182a8a2bc4079147a8580e23996d09d3c5e503644fdb539fffca124d166b729bc1df9d0eafe083d11f139d288e2e5dda47d75ffb370e1f871298d7fe58a355ee60a5b01d4015f016dcad02701fe71d65212bfa81ea47cc360d4948197a8b383b429ab8ef0566405000059000000000000000500000000000000")[..]).unwrap();
            let next_validator_set_inclusion_proof: dto::StorageInclusionProof = Decode::decode(&mut &hex_literal::hex!("80fd6e027f7a1bd8baa6406cea4d80d9327120fd2add6d1249bf1b6bfc3bdf510fcd4edbc19c4bcd11a987f6af78373b6308c6fb18a9a91c75a6373daa6c6ffe825d189115e08a8c0539ae97adbc7314a94a70c5021082fb53c694a973078eb507cb973e5252159510966c31c327987893ee3f33750c306b7142734b1135a6095e43895854100642414245340215000000c904e122000000000442414245092601f0380d397b8cb68efff0eb5735a1f6c326ba2ae1fc83968f6b53a520ca9a3cbc660100000000000000a410fb1d99d465cfc9c866d1a6bc22cdc373a65a5c6949c835bb9a44ebb8ef6401000000000000003cbebc8c3c4898d3790cf3a4242cca2a408c2fbb9b5755407df16576e181f759010000000000000076ed00fa2455aaf0b51f815976181801ebfdc49490f02a93a28e5c6fb6c3311301000000000000001e952f9872a336c528654ec9af4552145a236c94ec0392d81edad0938ffb15640100000000000000c2c2e732bdd09b1a8912a3116b366463bb868288b3ce6e1cbe3e49b4e7e0ea0501000000000000000a0ae4d2cbd40094292938464d13acfe5e01feb0dc780745ba8febaad6b8193c010000000000000090c6fc566399ca3d4266fc8e588e4db33e0ed4c59759804d9d2015926063de1e0100000000000000ce7b1a2246d249145c0531154b22474e2c671efb19e39a917b6a20d25d955d4e01000000000000001ed4a596e597e1542d82f7117f44d35c8cc5acf054b9329b53a348a0805847570100000000000000606a744931d02911d947ce9e48fe0a87423e713bb2b8631c7f98ebe9a3286c710100000000000000da8a50cc9745af5e7e55257bbf9f1b0ef4bd08ec5c4b1b8d38106fd32f3cae0401000000000000008aa80106b9bf9543726996d571da80d31515b8b0e4e1433bba24541fd5803c2201000000000000000c19fbbf9dfee27e90421556427ecfb533147efee7cdcfd63a5e5605f35c7a7401000000000000001c9a8bb6b424510eec51389ff938cab6ab9bc6af0eaf400daeae09034cc7e0180100000000000000347bc0b3971351b1c4602ba043db7247f5e47b36be22e78fdb3215ba74671d4c0100000000000000b02da965e9786cec099601a3c41f81c26272b55bf3a05b2106ef96ca962c932a0100000000000000a0d6165a41f48d7eca14d4bdfcbee8b65b57d948494b39cab19715ece5e6ab5e01000000000000003cc7a84ca415a3188ae375976a560a4b2e84946aef2a5714e428304f01d59d620100000000000000ee0b3601ae2499ae47df26ad165e97137c05f45432538b38babe76b04232921e0100000000000000ba0fe9f2282505e8ba44e8376fbbedcd94226b8668dd7d4944e4ffa7835b760e010000000000000058087ea312fce332c695b1a0ecaa6ff92a434b0544032373ff76681a73c82d3b0100000000000000f67d0cd0496f4e6b63d5cf44b2d81aa435b2b301b2518904b10da490d0521d0e01000000000000009ed3c115a1ae4bb5deb89e4704fd5257354f9ade790ef964953a61c0a7bce3120100000000000000c2a43f0e3ffe17327b0ceebd7a5271c43d7196be2b5e5231c484739b5c30ea6f0100000000000000f8aee915f4fd2619c24438145982b9b80b4d1ef21caea4d78850b928802bbf720100000000000000863f9af1c8e857a131f9c1b6e4081eea8a2bd3479344d900176ac7fe00fdd54901000000000000006acddf5d0bfafa9192129394931bdbd8ec1eb7c9789130b9da7a44b2d8c17107010000000000000022919327b2980b02f874ace62b3667dc84250aeb6ad17e0532ecf20f7a893e130100000000000000ecb83703aef5fd75d176ff7ae5cb2b0b8cb7c6d09c689266712c51c87db1784801000000000000004a6d1bb5dfba6e7a40e6e6a1410c6cb849c6ec955183eed7360d1324eaab58330100000000000000f209388de0a83c211e88f99451b4e72443e2b1a43ca5c2a19e8b20b938c337250100000000000000983d61337fc5ff941ea2522981d01ae4e652b78cea8606e8b99a044af604013f0100000000000000a2a3d6d5de5b0eeef375d1875c825b2c9cc15850b45da1c5987b9a181af2630e01000000000000002650e2fe61dc584911552338cc38e01e94181ed7491ce5b4d80ec12404bce93401000000000000002a1f7669434fe9c2d64c4a75ff25979e2536828323b6df0b1dd36a5ddb98e84e010000000000000020ec9497154712bf4fe5b2c65a1082685d8dcfaab2fc52239e5064eec366d7130100000000000000b6610d6baa2a5cdc582ca6a59e04b81cb4b8217171a8ec65953e66c28b49af35010000000000000090a09621a611cf403f401e61606da4fd7e4da75c844e787c3677699f39d9ba3b0100000000000000808178752fb5bc2d7ad6d9606892baa9a6cdadf1cbd8734228b4dc8517540712010000000000000098041eebd9dac1433a47655e2e8b8a1b09f2f901182ddd9a506c398ae9a0c47c0100000000000000d6cabdd9eedfdacd87aa65e7e45c709d2e1272317309128e7fd01de2f1dbf1150100000000000000faadef82c6a2822860afd1e1a49fef73b407d7bc67518086bf7e0eba12617c7e010000000000000040fd58bf5c39ae92339afc37ab17980000a33cb66747e3be472541a63c0a5c7b01000000000000008268f51c55c77d48f901d48fcb01bf2e7ff1d70ced125fbf17d4313f38c1866a0100000000000000bed56a1855b837e61b13f2bcba90e348c10759433aec6a9769eb44124ae60f6401000000000000002ceb89a06fd64ef2af7d784df5df6b5a48ceb73fa9030f58bc8167829bbc67460100000000000000b0cca2b4bca85b530b70f03df31336e5993bccc5affc7dd262bbd72fb68971020100000000000000049355c8e5ec54f0f96cf239a233ef1a0706153ee6fdb12d48009cc3f5940b630100000000000000284d01879de483c1eae463426c64c0dc0a7e6c420b7c0c58f27a6659a85bce5e0100000000000000aaefccbf988f905a828ed6b7ca125a55daec4fc51d4e3cd2532e02b2fabab47d010000000000000072750851187e1e94d2c928ec2bd37ecf7bc1f768e69a8ea4577cca8faa7d307d0100000000000000ee5e9c468922fdf80c0ced4ff9a1f17381e69d85cd3c1a70a4fe5f0aa3b4ac0b010000000000000004ee1ebf7dbfc0aa5df51c7e1d7accef6131dc55f0e0596b8cae876eed3570760100000000000000ba28c28e2664419ede221d34c075b0746881aa3bc2d308d5f569e0f832c6d2550100000000000000005428217cd90c39504c2ac004637d57983033be807f0c4f1d7e13384502f061010000000000000076f47ee5f017237c74a306d586011a966791a9338adb5e002058e221a1eecf060100000000000000dcb1dc506e297fc76d09f02e111861af2bc031427176c241003e6c127b4bb2640100000000000000480d37472d32b8d7ae17c7ff220c502e68a28296b8730fcf93dfa28f2201115301000000000000002a6d729043962c310c5b121c7cab5b8f2051149bc804ba38ab2fffae71949f28010000000000000007d17c8ed30e78df7ccae2108c83339198085eb266f45f45f9c26d7489d020710446524e4b992501f0445f7d52a0405ebf12f1b69cf1e7637c6f04a7d67e1000df3357894e5bdb294d0100000000000000c2252920ec60963094a935b6cfc483187d4092fa1d4af6c17a8fa93d90ffb8260100000000000000c351f301f71458df98b139511ecd8a9e4e0d1b18471a4075ac6ae8c02443bb2e0100000000000000983bd8c44f9d09d1f17bfc97fc494f0100d054f50c927cf35fd71f0df7788a7f0100000000000000e0c5b5ec1b83df6126744a6bff0cd76ab7c046997f1d7e0d6b4ce62133b3b35d0100000000000000b5af6489c3676b81365bd07bbc40c41833c8dca8cf433287805fcd3bbb0d35ee0100000000000000e6cd4fa4fde4ab4313a607c588a97d4dcea63fd1265cee969d43148fcfcba89f01000000000000001aee03d498efa7f271afabdd9997e1c393bfa345a88525e6dc66bea95904c21601000000000000004f5f8c8ecb586670a7935c4395eb85d3072311f5b1942e6bc2d658e7b835dc1501000000000000004bf75538620f9ca48a473526c95e4fc0c68f2ab37e3cf3f86db4e70145b85b180100000000000000f19aa7b82162a73d6685224ea61bccacb80a0db0bc172b560edd9969bd06021e01000000000000000a5cd87a6f76c0f6ce984a92ffa0379e01e0c9b87c7bdbed4aea25b76b3f0621010000000000000076d3a063b9d141719b87c9dae10a52a94b5f8818d1bd12f41308a7654bb890450100000000000000c56ba044a5f3e9e4541b1de73b75db78c22838eca8a7bffe4f9507dcfdae357801000000000000003ff89a98ee0654affbf2fd1c97d9cb4aab58e96855ec13bea0a88c06ce4ac05f01000000000000008e6c6bac12f96d9b8ac1e021e028ad0d20a0056fb4c0df6b8e0c45ef53779fe101000000000000002c5950454cc20d515cda4651ee54756889d8d63b8a80f640402779a8ccf03ecc010000000000000006fcca0083a3bd75816f7a3a648d60a93188802bafa3d11653d66a9dae47c57f0100000000000000445f86c96680f0a9a243a8d14e14f718ac5f801b713df816dc1a0b7481e2b3720100000000000000fb44a7e8d87632b76beeddf1cbee936661c8a537c5bc2d1e71605b18592e16f9010000000000000024c87f0e8b3f708353c643c1c09b9ac58bfe315f806f40bf8b2eb2e4ec359083010000000000000056b818ed393608f152726e8dd0e1ce1a0b9ed324b0ded2f5d37556adfa762b5d01000000000000004c34faf776208e4e4875d8220e14a2719f407b029ec0904d6064df6f58c2b077010000000000000076234a0270d2ce54912e28b9f2edfc2e214b23c12fc5a0b10e13d916eae36ae40100000000000000ac362195f8ee28f391a05f69c5a48384a669be9c1399fa864090af4f301c9d8b010000000000000072321606783f99157d2e719dab6023f4fb806bdc6314b0c713685885fd33b2930100000000000000fc0ae85ef08f985831239df655f682eb37df2c1a3e79ac57365d18585182a8a2010000000000000020b6ae8c48d06bb371a0a428d4f7e77c02d527b807e4022b67aeddfc003b85910100000000000000d2c55bd8590fd86e044d4b2e71a3d79e95e76d8a2bb65c0b9203e5a213b39e940100000000000000351489d655c589e5fc50284e1a23e86cd6a0268c43eb33431a6d16258c91d18e010000000000000042f213715f9f5cc59a386ea5ccb490e05a1adb37aced8352554ad9eaed8f01600100000000000000ed55d07b574c70e58eb739d0de41908126b226db0bbb8046e117e4226cca9a940100000000000000d654f1929675f18f1f5f0b621d7fcfc45cfd8fc1c5ec6003241b04bb90caed5c0100000000000000f90e0a1957fe389f08ce7a359c773b1d985f9b9856664a0ebbe7bda411a3496801000000000000006b08bad5a20a2f41c15a3c50adf211a0514801c84b93a81c5736d580c35c9b970100000000000000814d803dddfdd80f704ef47bc1006b56430a8db3a98b62669148f8ab7ef506c201000000000000005c05d8aaaecb537cf803d82dc32945069ea49f4c9487ff975336c8edd0c14300010000000000000041a847b5761344b4ac67b06162b9036b23e634f63a56fed7c50c253c5bbc68c30100000000000000613c146bebfe289a235e49b63288095813c139b4889f2ecf5168e0ead9368a02010000000000000090550cc052127c98437141d3c757dbb1e7035780c017fd58aa261f48c1cea94601000000000000008725c0d242f2e314229c1912befaee704de9168add534d7ccff2e43a2666a8cd01000000000000003e5290bd625d85f44c83fc0ee203a70d9f080e521bed93c38074ec210fffa21b01000000000000006cf476b617fe265f71945aefc0cac2d25e6143439d6e1126fb53a9f782419c7e0100000000000000c0caa7a4a1b3cc2c4f16de8fdc4d617dd649a31b45273e5b3f055cf771ef08450100000000000000308578649e1a0c3c00fb35b8a5b954fe46b5f3216bc0034449f87b4942fc2d6001000000000000005267e8ed5c4dd7f49eea561b8206cb8f177f402168efc76e4b20a6a8789e10ea0100000000000000f59b326c7c1542c8f47e968549aa76c7c12e22d8e1f7aba719bd046829836ecd01000000000000005bee26f7648b838abec1d5397ec3427134a05ac9f5998a6e0a7144a41ee5009d0100000000000000ca3af12c6a83b6755e7303d1a7c9a69e8fd5ec37d74ff21fb6a0d2b0c15a42ee01000000000000001d10169c1f8200f4d26745888de07134b9b847d0c52789e6ca8a41593d1a6bcd01000000000000001cf3d1df89bdea25e0f1c61038e351aa01ecd6ab6d4b316d858898a3671da25301000000000000008f2d020d03fe65bf758654b074185b85d9b15f9c60f682b941eb8ab6de67b25801000000000000006d2ca041d0171dfc9fcaf9945ac4f284541ada0b7c0c70be795db80309d0564a0100000000000000129eb2020980cbd6cc737239f914d6e001298dc0b8b719ed233f2456e90f1b1a01000000000000001a3cae6630d6d99a01a4699cd19ce878bedfc9cbdb752200deeb6bf2c1cf96e70100000000000000f76c6eba4aeb656927fd5fade7e45732544ccf7d7ef08b76c879b87adc7fc6dd0100000000000000ae93404d6f8f5d90d75e48f9d294a945335b8b8eb32d5343ee52e4f7b78bb3af0100000000000000cd4ca87c974eb1f85da891c1970f177641b5e0f74629f3b30768242f862334b401000000000000007b3c2fd388ab16d628137b021c633a26aeec49b5f45490e88b75040a95a1f48d0100000000000000a9898572ec21113b5bca00e1287f8ad3820ca36df9260be5426a2af391b4c855010000000000000000000000054241424501012ac452f51e7359c4a427f39a1fe8703f442f07cbb11abcc78b5b0f209fbbe068c9cbf92946cc61e1b2cf78c8e4957b2313ed46edea4b57f81d0333d64e0608830ca9029e6e027f7a1bd8baa6406cea4d80d932b0a080b8e055dec166f62102fde0cec5b354ce9ed63f268552fdb40be6a3094ff542c54c5f01d47f736dfc5807b7a3033e859a8dfd0400803837f9e673ee9fddeac3c3725bf6a47ee972f8a6a02ece883bd3c935152986d680d16dad8d5b79c031830ae4c85dd12ffe63923aa58a2494ad269b85a5a5d331aa8067485462d5200506f662399fb390026a27505f90d08ecd2915aa2ce5966fc8a407990180016080d52340c44506f7c9d2b6fe352c3edc539600062cddf849f8498e5f8e7122c3d2807f0ee35613089581a06ddd04a3ee2ecaa0b66e4700383d7c87d5fbc182cdcf8280fbb0fbeef4e03769226c714de9ec6e3a6e011c24eb44cea81c31ea3e68b3b5390dc10680bff58039f6069ecb5d3e0d0eceb324c65ff316659261b1a200d1e398f79c70d5ed6930805d00d10433f41e3fab8e48448d08ca2bdc2d13e7f0ffa1656a82f40e498cd05380108694a189b7325b3869a4713fd18de34b2e3fa69c93996903848cf07ee8c1168088bdb0cac5a5f2104187c6265fd56fe918e9d01f1419d81c634b8edc310d4ca68093b4202075803222e5bd965576d23faf002c576dd00a17491ec27a6a2c511a7e80e459543381b60e41bc283704850132026d2d44aa183d7bfe9c3fb7c4601fdc708068264e6e29190379643fbb028141335583bd61465883da1aecd0d6ec638539e2803554e8c619c26e2bb67aafdce9d55552e464378fb77745b63390af3af6db79778004eeedf1d3c9dec83c629a44c467fee3a8a7dba7200b5aaac906a6da25db3db680d499f182295113c7041563caae5e623503bdfd3b270a3dc6d1cfea1a48f43b5b80d54d6bacbfddc26d090dcd0988ceb949e2d9b92eb00260e77f827c19f7dd9ba880678d5fee737de3237bc2e4e2bcc92fa659b8cc33ef37cac215f1212311bb7e4a80e6a2cb1709bf7b0f4b01ff4ffc90b06f87e86b03784c73148964f591426602d80fc85f0120fd2add6d1249bf1b6bfc3bdf510f809c1b1c4b1cee0aa024fcb364baab4e3274cd40e69fcb7bcd2062ab062af41796809c1b1c4b1cee0aa024fcb364baab4e3274cd40e69fcb7bcd2062ab062af41796")[..]).unwrap();

            let next_validator_set_storage_data = next_validator_set_inclusion_proof.stored_data.clone();
            let next_validator_set_inclusion_proof =
                parse_rpc_inclusion_proof(next_validator_set_inclusion_proof);

            let now = Instant::now();

            let proof = prover::proving::prove_validator_set_change(
                previous_proof,
                parse_rpc_block_finality_proof(current_epoch_block_finality, Some(thread_count)),
                next_validator_set_inclusion_proof,
                next_validator_set_storage_data,
            );

            log::info!("Recursive prove time: {}ms", now.elapsed().as_millis());
            log::info!("===================================================");

            proof
        };

        // genesis block = ee90a215d19f2863b3b764bab45c1fdc7b4500a38c658368a5ba2b2ff977a20a
        let current_epoch_block_finality = Decode::decode(&mut &hex_literal::hex!("f0445f7d52a0405ebf12f1b69cf1e7637c6f04a7d67e1000df3357894e5bdb294dc2252920ec60963094a935b6cfc483187d4092fa1d4af6c17a8fa93d90ffb826c351f301f71458df98b139511ecd8a9e4e0d1b18471a4075ac6ae8c02443bb2e983bd8c44f9d09d1f17bfc97fc494f0100d054f50c927cf35fd71f0df7788a7fe0c5b5ec1b83df6126744a6bff0cd76ab7c046997f1d7e0d6b4ce62133b3b35db5af6489c3676b81365bd07bbc40c41833c8dca8cf433287805fcd3bbb0d35eee6cd4fa4fde4ab4313a607c588a97d4dcea63fd1265cee969d43148fcfcba89f1aee03d498efa7f271afabdd9997e1c393bfa345a88525e6dc66bea95904c2164f5f8c8ecb586670a7935c4395eb85d3072311f5b1942e6bc2d658e7b835dc154bf75538620f9ca48a473526c95e4fc0c68f2ab37e3cf3f86db4e70145b85b18f19aa7b82162a73d6685224ea61bccacb80a0db0bc172b560edd9969bd06021e0a5cd87a6f76c0f6ce984a92ffa0379e01e0c9b87c7bdbed4aea25b76b3f062176d3a063b9d141719b87c9dae10a52a94b5f8818d1bd12f41308a7654bb89045c56ba044a5f3e9e4541b1de73b75db78c22838eca8a7bffe4f9507dcfdae35783ff89a98ee0654affbf2fd1c97d9cb4aab58e96855ec13bea0a88c06ce4ac05f8e6c6bac12f96d9b8ac1e021e028ad0d20a0056fb4c0df6b8e0c45ef53779fe12c5950454cc20d515cda4651ee54756889d8d63b8a80f640402779a8ccf03ecc06fcca0083a3bd75816f7a3a648d60a93188802bafa3d11653d66a9dae47c57f445f86c96680f0a9a243a8d14e14f718ac5f801b713df816dc1a0b7481e2b372fb44a7e8d87632b76beeddf1cbee936661c8a537c5bc2d1e71605b18592e16f924c87f0e8b3f708353c643c1c09b9ac58bfe315f806f40bf8b2eb2e4ec35908356b818ed393608f152726e8dd0e1ce1a0b9ed324b0ded2f5d37556adfa762b5d4c34faf776208e4e4875d8220e14a2719f407b029ec0904d6064df6f58c2b07776234a0270d2ce54912e28b9f2edfc2e214b23c12fc5a0b10e13d916eae36ae4ac362195f8ee28f391a05f69c5a48384a669be9c1399fa864090af4f301c9d8b72321606783f99157d2e719dab6023f4fb806bdc6314b0c713685885fd33b293fc0ae85ef08f985831239df655f682eb37df2c1a3e79ac57365d18585182a8a220b6ae8c48d06bb371a0a428d4f7e77c02d527b807e4022b67aeddfc003b8591d2c55bd8590fd86e044d4b2e71a3d79e95e76d8a2bb65c0b9203e5a213b39e94351489d655c589e5fc50284e1a23e86cd6a0268c43eb33431a6d16258c91d18e42f213715f9f5cc59a386ea5ccb490e05a1adb37aced8352554ad9eaed8f0160ed55d07b574c70e58eb739d0de41908126b226db0bbb8046e117e4226cca9a94d654f1929675f18f1f5f0b621d7fcfc45cfd8fc1c5ec6003241b04bb90caed5cf90e0a1957fe389f08ce7a359c773b1d985f9b9856664a0ebbe7bda411a349686b08bad5a20a2f41c15a3c50adf211a0514801c84b93a81c5736d580c35c9b97814d803dddfdd80f704ef47bc1006b56430a8db3a98b62669148f8ab7ef506c25c05d8aaaecb537cf803d82dc32945069ea49f4c9487ff975336c8edd0c1430041a847b5761344b4ac67b06162b9036b23e634f63a56fed7c50c253c5bbc68c3613c146bebfe289a235e49b63288095813c139b4889f2ecf5168e0ead9368a0290550cc052127c98437141d3c757dbb1e7035780c017fd58aa261f48c1cea9468725c0d242f2e314229c1912befaee704de9168add534d7ccff2e43a2666a8cd3e5290bd625d85f44c83fc0ee203a70d9f080e521bed93c38074ec210fffa21b6cf476b617fe265f71945aefc0cac2d25e6143439d6e1126fb53a9f782419c7ec0caa7a4a1b3cc2c4f16de8fdc4d617dd649a31b45273e5b3f055cf771ef0845308578649e1a0c3c00fb35b8a5b954fe46b5f3216bc0034449f87b4942fc2d605267e8ed5c4dd7f49eea561b8206cb8f177f402168efc76e4b20a6a8789e10eaf59b326c7c1542c8f47e968549aa76c7c12e22d8e1f7aba719bd046829836ecd5bee26f7648b838abec1d5397ec3427134a05ac9f5998a6e0a7144a41ee5009dca3af12c6a83b6755e7303d1a7c9a69e8fd5ec37d74ff21fb6a0d2b0c15a42ee1d10169c1f8200f4d26745888de07134b9b847d0c52789e6ca8a41593d1a6bcd1cf3d1df89bdea25e0f1c61038e351aa01ecd6ab6d4b316d858898a3671da2538f2d020d03fe65bf758654b074185b85d9b15f9c60f682b941eb8ab6de67b2586d2ca041d0171dfc9fcaf9945ac4f284541ada0b7c0c70be795db80309d0564a129eb2020980cbd6cc737239f914d6e001298dc0b8b719ed233f2456e90f1b1a1a3cae6630d6d99a01a4699cd19ce878bedfc9cbdb752200deeb6bf2c1cf96e7f76c6eba4aeb656927fd5fade7e45732544ccf7d7ef08b76c879b87adc7fc6ddae93404d6f8f5d90d75e48f9d294a945335b8b8eb32d5343ee52e4f7b78bb3afcd4ca87c974eb1f85da891c1970f177641b5e0f74629f3b30768242f862334b47b3c2fd388ab16d628137b021c633a26aeec49b5f45490e88b75040a95a1f48da9898572ec21113b5bca00e1287f8ad3820ca36df9260be5426a2af391b4c855a80a5cd87a6f76c0f6ce984a92ffa0379e01e0c9b87c7bdbed4aea25b76b3f06219daf0dd1902b9f2be724a5be1ce355107c2ccd5f922b6a23bcada496a3a51f6ffa88c73e4bb91aef4a2820a469ff63130e3314114e2fcd694a25379030394b041a3cae6630d6d99a01a4699cd19ce878bedfc9cbdb752200deeb6bf2c1cf96e70843268ff076e20de1dad1f81eef077d98f861fae2ae68064b1c6cacbdfa0290516a36b92a787e1fbdcee743fb80460d8074ac06c3ec335860da2b0f38eb43061aee03d498efa7f271afabdd9997e1c393bfa345a88525e6dc66bea95904c216b9e225bb040af6faeb6408b56fb2354c67ba01d485e62e7b01e2cedae705adbc89069023e03a4fc967421176e59f5112ae9e28788fca971de5ac5d99e17a8f061cf3d1df89bdea25e0f1c61038e351aa01ecd6ab6d4b316d858898a3671da253a312879b5f4a6c1379c97bd0175f3a5115cf26af0e6f6cd3e9e63ab46c2be30fb84f38184f1c9ee54c853eef0f742d7bdd7b4edbc88c2ef09659900e3addfa031d10169c1f8200f4d26745888de07134b9b847d0c52789e6ca8a41593d1a6bcdf7f8f6441ba4fce135379b4b3f7706f456f6bfc617b1ec4b5eaf9ab2e69fa0bb79b256378b080d4c254d33feb593d7a7f79adfdd396ff89bc7b083952547b20724c87f0e8b3f708353c643c1c09b9ac58bfe315f806f40bf8b2eb2e4ec3590831832d3fa484d5ca6cbf51ad3f2a1905fe7a09d0d73cb9514844305550a301cce2b0f7c17a623f9ee0efb04b32f0217cb8fed1673f59b7d62f5ab1f1286f7710e2c5950454cc20d515cda4651ee54756889d8d63b8a80f640402779a8ccf03ecc0fd0933f345c6cf5efa23d11c70a32d16dde93331418289ca6364b8528963eebfbcef53c5478eed3ffc2fa87f4e6cc263b7f2a859e2e93024a062838b041a908308578649e1a0c3c00fb35b8a5b954fe46b5f3216bc0034449f87b4942fc2d6053fa981dcfee595f57d9e43a3bb4bf60b94033cb3c4344e6df057ea42d684d4a8c993d71484373d5f49b93e8af0189b7c47976e42d992ab91e441c9ddd0fe007351489d655c589e5fc50284e1a23e86cd6a0268c43eb33431a6d16258c91d18edeb12a5d49b39f3cefe1f919336e56ec2b2f19de6d148e2e1a9b7b2e18abae9bb7de33093cfe6708686bec29bac3a831ba2b554a59b8da38ef3fe7e5588072043e5290bd625d85f44c83fc0ee203a70d9f080e521bed93c38074ec210fffa21bd7ecf765c82d66660057a9855d1c9313a3ce687c2f980691cd848cea4d433861f8a7db35136a0f21a6c123ee68b10722d3e0e6c3440371f7f0f15001f9adfe0b41a847b5761344b4ac67b06162b9036b23e634f63a56fed7c50c253c5bbc68c34e6f4d54d1dff888f97fe45a5823cfa7f1b12d06676cbffe4e738557fdf59493f8ac17fab4fe4b8727f8fc199e7675a47dd3f2575da5839cbc93bd9c354bae0f42f213715f9f5cc59a386ea5ccb490e05a1adb37aced8352554ad9eaed8f01605a71819b2d4216b79ea8d7969edce6a9d0cb964a5979f034826005c8ef18fdc30d38fd2db49ed39458f9ef59ea792ae91c2b4302227d152e9c29c50f56088101445f7d52a0405ebf12f1b69cf1e7637c6f04a7d67e1000df3357894e5bdb294d516acead21bfc04ab2ebd836c10547cc2cf699b29d0002ee7c44cc930446deb617209066a5137e20ca7ea7712bfa9ef2a63b61a0426539f946057e0c6622880e445f86c96680f0a9a243a8d14e14f718ac5f801b713df816dc1a0b7481e2b372b6010142fbe430a921fac6eaf1cd8e773d6e9d7370a2576f17fe5ab0570d4c21bdae3612d7e8b17e175de478e5144516d35dde71aca2dfcca286749b82bea1094bf75538620f9ca48a473526c95e4fc0c68f2ab37e3cf3f86db4e70145b85b18cf2f21ebbc616e3ea1b72c6bccd807d1149d97ea5e028b9cf3f54aff458ab1fe4b91014a0049ecbf0a566d202a893cd80557d4e7549456f9ac1e7c022bb0c5074c34faf776208e4e4875d8220e14a2719f407b029ec0904d6064df6f58c2b077c4d9a1e4352d210205aa68d9013debee7b79854c20f190d6232f1a54ad4d083279495f37679b37893a0ff032f3cded723a96f0853a6b73c7676a578d9470e1044f5f8c8ecb586670a7935c4395eb85d3072311f5b1942e6bc2d658e7b835dc1562c75560f4d24d74efa61d34e206de19ee6c104401de5b0f9a3124fae7a0abe8cd104bca43185de492591b998459c735d69c2c4ac61817775a5f377bfba071065c05d8aaaecb537cf803d82dc32945069ea49f4c9487ff975336c8edd0c14300f58e9841724a31ad59fc93eacd83751bc4714ef80549ec1234889cc3e6a0b917412da4c919396ded10d4a65418b589cd1062422b1b3f1318a27c959233e1af0d6b08bad5a20a2f41c15a3c50adf211a0514801c84b93a81c5736d580c35c9b97a3a752ceaa4d8c4368be69288889f9648ffa10126dc92c8e4d2d3696ff5acaf7b8564a045ae99f554c34b88f18a2dd587c934432f922e81cc87f6b3aacf85e026cf476b617fe265f71945aefc0cac2d25e6143439d6e1126fb53a9f782419c7ebd9b94dd0c22d3c5dd25bef2107e264635189cf34ec02c4b756283352e54f8929b5c7db02ecdeecaef22e28dea0497267ef960d731f77d74e8a6ac6e365dda0272321606783f99157d2e719dab6023f4fb806bdc6314b0c713685885fd33b2933ef209c204ed3646f895f440520bef29dc5d2329b6a2f5c34f7f6d13a9c477fb0f6aa846974769ace3fce290a53dafee61b89a3743ecac26d7f648fdb576780376234a0270d2ce54912e28b9f2edfc2e214b23c12fc5a0b10e13d916eae36ae4e0e92c6953e6780262f499196139a757aad84a92b407adf513eb5356a0e87eef5aa5eff4329c0f2ea5a73b3b5d0899c1af57655ccfe6bc6603b7654affb3fc0876d3a063b9d141719b87c9dae10a52a94b5f8818d1bd12f41308a7654bb89045d7c9c76856a51f8fc838d97080d1433f15d5d0313f1865430974a38272b4218f7bad979d8d61860031185a4e6e9ef2f9ff15ffe6094220659e669dcfabdd910f8725c0d242f2e314229c1912befaee704de9168add534d7ccff2e43a2666a8cdc5302737b8b901b2e950f2148e18e3ae7a2c4ebafa1d7e23e2a0c4aefea48953067846bdad4fb8e6ffce7096ff6d92f15cd51dff139f5d74d5be97ae9a858b048f2d020d03fe65bf758654b074185b85d9b15f9c60f682b941eb8ab6de67b25811fd4a42b78037d69943bf9d25e615158ff59db7e406c882efd101e8dcdd0218f55a41f2d48ca168d2d45625e6a2e8fafa8d1fe20913b3cb5df8d9632bdccd0b90550cc052127c98437141d3c757dbb1e7035780c017fd58aa261f48c1cea946e2a3c2523ded0c3e2fdd48fb6dc9918a853860e6d54f28e97f25826e38cfe3a7d7b135f2945e95795108a538653a1aa9c9058deb0c476eb105ff52a96b84bc07a9898572ec21113b5bca00e1287f8ad3820ca36df9260be5426a2af391b4c855ecc2224df807c2ee46bf7da512a6febb4406ab4085a70663dbf627a1a3792e098e3212b4b8339e05c277cbe67d5a38caa584cab02ebdf2d59c88e9868b679500ac362195f8ee28f391a05f69c5a48384a669be9c1399fa864090af4f301c9d8b261ad62c9df492c02be11f992ba5b111aa36f466fe530f9e72950e13c4a1e74f533214ee520b12ce9c1ffb8ad1982f67ffbc7ea385ed39bd8f564e66b33a8301b5af6489c3676b81365bd07bbc40c41833c8dca8cf433287805fcd3bbb0d35ee886d207ce5c76b780c9f43b894309781bee49ea60e2df9d2fc8a97eb1f431df0f99d22650746e3c61c0e9d6ffcc76cb835cb07a4dd0d330e9a25c30bbb8c6805c0caa7a4a1b3cc2c4f16de8fdc4d617dd649a31b45273e5b3f055cf771ef0845decdd39e13c3c49377c791f8d9439707db67af669f026c1f90dfefe01598a3a71b3b659e975fda563d7819d0471eaaef64c612d3472fb82751715ea8a17c920ec2252920ec60963094a935b6cfc483187d4092fa1d4af6c17a8fa93d90ffb8265fa58e880bde1cab9e2e2aff03dd65fefcc4d37710d656ccfea55e819bb70ddb381569388da35cb6be37508183cf3f855e28851da79736edbac78ffbfdcf2208c351f301f71458df98b139511ecd8a9e4e0d1b18471a4075ac6ae8c02443bb2e44b6df52c508829761173b90788f62d0e66ee935353af1e962963a3005397f0e612e56440ae1ac16ea5e608dc09ae18a52cbdcd953b438e3fa39b0740ba5f60bca3af12c6a83b6755e7303d1a7c9a69e8fd5ec37d74ff21fb6a0d2b0c15a42ee17880cf3bcd8e14415051641a785551b2ac79205012ed4274c9bedc5461eff89c63b6a60d7d839f421a23381d32d25e204b78ac161476032eac67de8500d9009cd4ca87c974eb1f85da891c1970f177641b5e0f74629f3b30768242f862334b4d3210a041f3207bfa6ff072b6feb7de536a94bc4894d8d847269137aa85a3457d83cc19b45d43162c1968227f219167f5c4525e8a0934c24505ac2961cb55305d654f1929675f18f1f5f0b621d7fcfc45cfd8fc1c5ec6003241b04bb90caed5c1dba827913ae47b8b11c6b74c69368070b0b51218c38c864ce9942cb2a152fb2142c6a916dfa75ac4568e6591758e3576b40d5e78bf17b7e4101dc4d6f4bfc0ae0c5b5ec1b83df6126744a6bff0cd76ab7c046997f1d7e0d6b4ce62133b3b35d983815534a1a16083bb4096f63cc5342adc1043607d7c19310f69caf54794e6a71f3a9a4d9031c2454c6a1e87b0975252f591884bc42c4c9155693b1e2fd5c0de6cd4fa4fde4ab4313a607c588a97d4dcea63fd1265cee969d43148fcfcba89f02d8a44ad6f15dee1964af72de29df3288ff6726c9ae3cbb3ce6e7466fe0caf51e90d2ad5da8a0f453fac61fc66bae0c59138473f3d24827ff84e57e9d494007ed55d07b574c70e58eb739d0de41908126b226db0bbb8046e117e4226cca9a9417a64f124823109bb569cd84713aabb5ab584d6cbbb5671da334db63b45744297aa4bb6c4908cc9ead5ad42db6069601b85a7e38c0c7bbdecddc82a29592110bf19aa7b82162a73d6685224ea61bccacb80a0db0bc172b560edd9969bd06021e6fac19b79739b9d6ae39f9700d536e77acb7e7c2e5c121a899f1d7bbdec45b97bb11bf45df74f6f9e8e59776d1dbb8fab9373e02f9df47b2edacd6946993290ef76c6eba4aeb656927fd5fade7e45732544ccf7d7ef08b76c879b87adc7fc6ddc5206f935727b6f6209c5dfa79747eb1fc899dc095b4a0ab4ff53b1276bb21360fb28b068898125b477c1c302b64e67eebf8037dcbe7e76289028c0580a40203f90e0a1957fe389f08ce7a359c773b1d985f9b9856664a0ebbe7bda411a34968701b73bd0e0e45943c38bf0e9d029f591f72f0ba45139c2c0e7e6cba79d0a9571b506a07ae7f7e8a89a230076a8ad006581176ed021283a6f311d1491995270afb44a7e8d87632b76beeddf1cbee936661c8a537c5bc2d1e71605b18592e16f9d69ea7f011ff269bd6a6d9a0bd6ef184f32ccc2b7cc53237cdbb0943a41456d101b4a9fa7403fbd98340ee690c4285c43c491059a661adb3789a8232393c2708d401ee90a215d19f2863b3b764bab45c1fdc7b4500a38c658368a5ba2b2ff977a20adc0500005a000000000000000600000000000000")[..]).unwrap();
        let next_validator_set_inclusion_proof: dto::StorageInclusionProof = Decode::decode(&mut &hex_literal::hex!("80fd6e027f7a1bd8baa6406cea4d80d9327120fd2add6d1249bf1b6bfc3bdf510fcd4e5e53a40571499eededdfb44b2fb8d7ffebbe2a0565e8d30cb294df5bbfb5e98371171c8251a9a1a39bcf0848a13f8ef7e6cade4aeb80c5fd00be2b9bdd47096a106f2c1f0ad0fe0160db4ea245076e0ae89710f8f1db4c27d03714e161f9da70ea6c1006424142453402240000004105e122000000000442414245092601f0380d397b8cb68efff0eb5735a1f6c326ba2ae1fc83968f6b53a520ca9a3cbc660100000000000000a410fb1d99d465cfc9c866d1a6bc22cdc373a65a5c6949c835bb9a44ebb8ef6401000000000000003cbebc8c3c4898d3790cf3a4242cca2a408c2fbb9b5755407df16576e181f759010000000000000076ed00fa2455aaf0b51f815976181801ebfdc49490f02a93a28e5c6fb6c3311301000000000000001e952f9872a336c528654ec9af4552145a236c94ec0392d81edad0938ffb15640100000000000000c2c2e732bdd09b1a8912a3116b366463bb868288b3ce6e1cbe3e49b4e7e0ea0501000000000000000a0ae4d2cbd40094292938464d13acfe5e01feb0dc780745ba8febaad6b8193c010000000000000090c6fc566399ca3d4266fc8e588e4db33e0ed4c59759804d9d2015926063de1e0100000000000000ce7b1a2246d249145c0531154b22474e2c671efb19e39a917b6a20d25d955d4e01000000000000001ed4a596e597e1542d82f7117f44d35c8cc5acf054b9329b53a348a0805847570100000000000000606a744931d02911d947ce9e48fe0a87423e713bb2b8631c7f98ebe9a3286c710100000000000000da8a50cc9745af5e7e55257bbf9f1b0ef4bd08ec5c4b1b8d38106fd32f3cae0401000000000000008aa80106b9bf9543726996d571da80d31515b8b0e4e1433bba24541fd5803c2201000000000000000c19fbbf9dfee27e90421556427ecfb533147efee7cdcfd63a5e5605f35c7a7401000000000000001c9a8bb6b424510eec51389ff938cab6ab9bc6af0eaf400daeae09034cc7e0180100000000000000347bc0b3971351b1c4602ba043db7247f5e47b36be22e78fdb3215ba74671d4c0100000000000000b02da965e9786cec099601a3c41f81c26272b55bf3a05b2106ef96ca962c932a0100000000000000a0d6165a41f48d7eca14d4bdfcbee8b65b57d948494b39cab19715ece5e6ab5e01000000000000003cc7a84ca415a3188ae375976a560a4b2e84946aef2a5714e428304f01d59d620100000000000000ee0b3601ae2499ae47df26ad165e97137c05f45432538b38babe76b04232921e0100000000000000ba0fe9f2282505e8ba44e8376fbbedcd94226b8668dd7d4944e4ffa7835b760e010000000000000058087ea312fce332c695b1a0ecaa6ff92a434b0544032373ff76681a73c82d3b0100000000000000f67d0cd0496f4e6b63d5cf44b2d81aa435b2b301b2518904b10da490d0521d0e01000000000000009ed3c115a1ae4bb5deb89e4704fd5257354f9ade790ef964953a61c0a7bce3120100000000000000c2a43f0e3ffe17327b0ceebd7a5271c43d7196be2b5e5231c484739b5c30ea6f0100000000000000f8aee915f4fd2619c24438145982b9b80b4d1ef21caea4d78850b928802bbf720100000000000000863f9af1c8e857a131f9c1b6e4081eea8a2bd3479344d900176ac7fe00fdd54901000000000000006acddf5d0bfafa9192129394931bdbd8ec1eb7c9789130b9da7a44b2d8c17107010000000000000022919327b2980b02f874ace62b3667dc84250aeb6ad17e0532ecf20f7a893e130100000000000000ecb83703aef5fd75d176ff7ae5cb2b0b8cb7c6d09c689266712c51c87db1784801000000000000004a6d1bb5dfba6e7a40e6e6a1410c6cb849c6ec955183eed7360d1324eaab58330100000000000000f209388de0a83c211e88f99451b4e72443e2b1a43ca5c2a19e8b20b938c337250100000000000000983d61337fc5ff941ea2522981d01ae4e652b78cea8606e8b99a044af604013f0100000000000000a2a3d6d5de5b0eeef375d1875c825b2c9cc15850b45da1c5987b9a181af2630e01000000000000002650e2fe61dc584911552338cc38e01e94181ed7491ce5b4d80ec12404bce93401000000000000002a1f7669434fe9c2d64c4a75ff25979e2536828323b6df0b1dd36a5ddb98e84e010000000000000020ec9497154712bf4fe5b2c65a1082685d8dcfaab2fc52239e5064eec366d7130100000000000000b6610d6baa2a5cdc582ca6a59e04b81cb4b8217171a8ec65953e66c28b49af35010000000000000090a09621a611cf403f401e61606da4fd7e4da75c844e787c3677699f39d9ba3b0100000000000000808178752fb5bc2d7ad6d9606892baa9a6cdadf1cbd8734228b4dc8517540712010000000000000098041eebd9dac1433a47655e2e8b8a1b09f2f901182ddd9a506c398ae9a0c47c0100000000000000d6cabdd9eedfdacd87aa65e7e45c709d2e1272317309128e7fd01de2f1dbf1150100000000000000faadef82c6a2822860afd1e1a49fef73b407d7bc67518086bf7e0eba12617c7e010000000000000040fd58bf5c39ae92339afc37ab17980000a33cb66747e3be472541a63c0a5c7b01000000000000008268f51c55c77d48f901d48fcb01bf2e7ff1d70ced125fbf17d4313f38c1866a0100000000000000bed56a1855b837e61b13f2bcba90e348c10759433aec6a9769eb44124ae60f6401000000000000002ceb89a06fd64ef2af7d784df5df6b5a48ceb73fa9030f58bc8167829bbc67460100000000000000b0cca2b4bca85b530b70f03df31336e5993bccc5affc7dd262bbd72fb68971020100000000000000049355c8e5ec54f0f96cf239a233ef1a0706153ee6fdb12d48009cc3f5940b630100000000000000284d01879de483c1eae463426c64c0dc0a7e6c420b7c0c58f27a6659a85bce5e0100000000000000aaefccbf988f905a828ed6b7ca125a55daec4fc51d4e3cd2532e02b2fabab47d010000000000000072750851187e1e94d2c928ec2bd37ecf7bc1f768e69a8ea4577cca8faa7d307d0100000000000000ee5e9c468922fdf80c0ced4ff9a1f17381e69d85cd3c1a70a4fe5f0aa3b4ac0b010000000000000004ee1ebf7dbfc0aa5df51c7e1d7accef6131dc55f0e0596b8cae876eed3570760100000000000000ba28c28e2664419ede221d34c075b0746881aa3bc2d308d5f569e0f832c6d2550100000000000000005428217cd90c39504c2ac004637d57983033be807f0c4f1d7e13384502f061010000000000000076f47ee5f017237c74a306d586011a966791a9338adb5e002058e221a1eecf060100000000000000dcb1dc506e297fc76d09f02e111861af2bc031427176c241003e6c127b4bb2640100000000000000480d37472d32b8d7ae17c7ff220c502e68a28296b8730fcf93dfa28f2201115301000000000000002a6d729043962c310c5b121c7cab5b8f2051149bc804ba38ab2fffae71949f2801000000000000007ba717b44926d96ebefe92117a7e9228a3bb13a94044614755dd3498a90646b60446524e4b992501f0445f7d52a0405ebf12f1b69cf1e7637c6f04a7d67e1000df3357894e5bdb294d0100000000000000c2252920ec60963094a935b6cfc483187d4092fa1d4af6c17a8fa93d90ffb8260100000000000000c351f301f71458df98b139511ecd8a9e4e0d1b18471a4075ac6ae8c02443bb2e0100000000000000983bd8c44f9d09d1f17bfc97fc494f0100d054f50c927cf35fd71f0df7788a7f0100000000000000e0c5b5ec1b83df6126744a6bff0cd76ab7c046997f1d7e0d6b4ce62133b3b35d0100000000000000b5af6489c3676b81365bd07bbc40c41833c8dca8cf433287805fcd3bbb0d35ee0100000000000000e6cd4fa4fde4ab4313a607c588a97d4dcea63fd1265cee969d43148fcfcba89f01000000000000001aee03d498efa7f271afabdd9997e1c393bfa345a88525e6dc66bea95904c21601000000000000004f5f8c8ecb586670a7935c4395eb85d3072311f5b1942e6bc2d658e7b835dc1501000000000000004bf75538620f9ca48a473526c95e4fc0c68f2ab37e3cf3f86db4e70145b85b180100000000000000f19aa7b82162a73d6685224ea61bccacb80a0db0bc172b560edd9969bd06021e01000000000000000a5cd87a6f76c0f6ce984a92ffa0379e01e0c9b87c7bdbed4aea25b76b3f0621010000000000000076d3a063b9d141719b87c9dae10a52a94b5f8818d1bd12f41308a7654bb890450100000000000000c56ba044a5f3e9e4541b1de73b75db78c22838eca8a7bffe4f9507dcfdae357801000000000000003ff89a98ee0654affbf2fd1c97d9cb4aab58e96855ec13bea0a88c06ce4ac05f01000000000000008e6c6bac12f96d9b8ac1e021e028ad0d20a0056fb4c0df6b8e0c45ef53779fe101000000000000002c5950454cc20d515cda4651ee54756889d8d63b8a80f640402779a8ccf03ecc010000000000000006fcca0083a3bd75816f7a3a648d60a93188802bafa3d11653d66a9dae47c57f0100000000000000445f86c96680f0a9a243a8d14e14f718ac5f801b713df816dc1a0b7481e2b3720100000000000000fb44a7e8d87632b76beeddf1cbee936661c8a537c5bc2d1e71605b18592e16f9010000000000000024c87f0e8b3f708353c643c1c09b9ac58bfe315f806f40bf8b2eb2e4ec359083010000000000000056b818ed393608f152726e8dd0e1ce1a0b9ed324b0ded2f5d37556adfa762b5d01000000000000004c34faf776208e4e4875d8220e14a2719f407b029ec0904d6064df6f58c2b077010000000000000076234a0270d2ce54912e28b9f2edfc2e214b23c12fc5a0b10e13d916eae36ae40100000000000000ac362195f8ee28f391a05f69c5a48384a669be9c1399fa864090af4f301c9d8b010000000000000072321606783f99157d2e719dab6023f4fb806bdc6314b0c713685885fd33b2930100000000000000fc0ae85ef08f985831239df655f682eb37df2c1a3e79ac57365d18585182a8a2010000000000000020b6ae8c48d06bb371a0a428d4f7e77c02d527b807e4022b67aeddfc003b85910100000000000000d2c55bd8590fd86e044d4b2e71a3d79e95e76d8a2bb65c0b9203e5a213b39e940100000000000000351489d655c589e5fc50284e1a23e86cd6a0268c43eb33431a6d16258c91d18e010000000000000042f213715f9f5cc59a386ea5ccb490e05a1adb37aced8352554ad9eaed8f01600100000000000000ed55d07b574c70e58eb739d0de41908126b226db0bbb8046e117e4226cca9a940100000000000000d654f1929675f18f1f5f0b621d7fcfc45cfd8fc1c5ec6003241b04bb90caed5c0100000000000000f90e0a1957fe389f08ce7a359c773b1d985f9b9856664a0ebbe7bda411a3496801000000000000006b08bad5a20a2f41c15a3c50adf211a0514801c84b93a81c5736d580c35c9b970100000000000000814d803dddfdd80f704ef47bc1006b56430a8db3a98b62669148f8ab7ef506c201000000000000005c05d8aaaecb537cf803d82dc32945069ea49f4c9487ff975336c8edd0c14300010000000000000041a847b5761344b4ac67b06162b9036b23e634f63a56fed7c50c253c5bbc68c30100000000000000613c146bebfe289a235e49b63288095813c139b4889f2ecf5168e0ead9368a02010000000000000090550cc052127c98437141d3c757dbb1e7035780c017fd58aa261f48c1cea94601000000000000008725c0d242f2e314229c1912befaee704de9168add534d7ccff2e43a2666a8cd01000000000000003e5290bd625d85f44c83fc0ee203a70d9f080e521bed93c38074ec210fffa21b01000000000000006cf476b617fe265f71945aefc0cac2d25e6143439d6e1126fb53a9f782419c7e0100000000000000c0caa7a4a1b3cc2c4f16de8fdc4d617dd649a31b45273e5b3f055cf771ef08450100000000000000308578649e1a0c3c00fb35b8a5b954fe46b5f3216bc0034449f87b4942fc2d6001000000000000005267e8ed5c4dd7f49eea561b8206cb8f177f402168efc76e4b20a6a8789e10ea0100000000000000f59b326c7c1542c8f47e968549aa76c7c12e22d8e1f7aba719bd046829836ecd01000000000000005bee26f7648b838abec1d5397ec3427134a05ac9f5998a6e0a7144a41ee5009d0100000000000000ca3af12c6a83b6755e7303d1a7c9a69e8fd5ec37d74ff21fb6a0d2b0c15a42ee01000000000000001d10169c1f8200f4d26745888de07134b9b847d0c52789e6ca8a41593d1a6bcd01000000000000001cf3d1df89bdea25e0f1c61038e351aa01ecd6ab6d4b316d858898a3671da25301000000000000008f2d020d03fe65bf758654b074185b85d9b15f9c60f682b941eb8ab6de67b25801000000000000006d2ca041d0171dfc9fcaf9945ac4f284541ada0b7c0c70be795db80309d0564a0100000000000000129eb2020980cbd6cc737239f914d6e001298dc0b8b719ed233f2456e90f1b1a01000000000000001a3cae6630d6d99a01a4699cd19ce878bedfc9cbdb752200deeb6bf2c1cf96e70100000000000000f76c6eba4aeb656927fd5fade7e45732544ccf7d7ef08b76c879b87adc7fc6dd0100000000000000ae93404d6f8f5d90d75e48f9d294a945335b8b8eb32d5343ee52e4f7b78bb3af0100000000000000cd4ca87c974eb1f85da891c1970f177641b5e0f74629f3b30768242f862334b401000000000000007b3c2fd388ab16d628137b021c633a26aeec49b5f45490e88b75040a95a1f48d0100000000000000a9898572ec21113b5bca00e1287f8ad3820ca36df9260be5426a2af391b4c85501000000000000000000000005424142450101b61fc33cee190f57eaf59dce4747cb09c4682eb25b41f18102ce9bfed93b82783b8a80c1fd7aadc7d36317e700b067525eb3c97102be1d930cd2390ceee93a860ca9029e6e027f7a1bd8baa6406cea4d80d932b0a080b8e055dec166f62102fde0cec5b354ce9ed63f268552fdb40be6a3094ff542c54c5f01d47f736dfc5807b7a3033e859a8dfd0400803837f9e673ee9fddeac3c3725bf6a47ee972f8a6a02ece883bd3c935152986d680d16dad8d5b79c031830ae4c85dd12ffe63923aa58a2494ad269b85a5a5d331aa8067485462d5200506f662399fb390026a27505f90d08ecd2915aa2ce5966fc8a4079901800160808e87729402c73ecbe52d9857a60bb0c7598978335f2aaa7ce38a1ee9e584f8b0807f0ee35613089581a06ddd04a3ee2ecaa0b66e4700383d7c87d5fbc182cdcf8280fbb0fbeef4e03769226c714de9ec6e3a6e011c24eb44cea81c31ea3e68b3b5390dc10680bff58039f6069ecb5d3e0d0eceb324c65ff316659261b1a200d1e398f79c70d5ed69308052930f33d8cfaa06e51a780c8e3d84cfeab68b3530123e6c9611dc7c3a615f608034975bdc84c47948528f660c3246e02ca3e4a1d475345292b2535c86e79655c78088bdb0cac5a5f2104187c6265fd56fe918e9d01f1419d81c634b8edc310d4ca68093b4202075803222e5bd965576d23faf002c576dd00a17491ec27a6a2c511a7e80de0e9ac1d9ba5b55efdd13b3d4c983134f0d65e62d1d5a9b8bc2d4fdc20015168068264e6e29190379643fbb028141335583bd61465883da1aecd0d6ec638539e2803554e8c619c26e2bb67aafdce9d55552e464378fb77745b63390af3af6db79778004eeedf1d3c9dec83c629a44c467fee3a8a7dba7200b5aaac906a6da25db3db680fe29ea4abfd474efd7d9522af40747e24f266fdad7318caf4323e4555746563180d54d6bacbfddc26d090dcd0988ceb949e2d9b92eb00260e77f827c19f7dd9ba880cd16c4f360a1e10489b7136938278d1878c9fad055e5cbcf64f16649e9bdf8e98030b99b16637aa7f54458965f82ff26f268a6420e1a3a6ed2754fb634697fcad60fc85f0120fd2add6d1249bf1b6bfc3bdf510f809c1b1c4b1cee0aa024fcb364baab4e3274cd40e69fcb7bcd2062ab062af41796809c1b1c4b1cee0aa024fcb364baab4e3274cd40e69fcb7bcd2062ab062af41796")[..]).unwrap();

        let next_validator_set_storage_data = next_validator_set_inclusion_proof.stored_data.clone();
        let next_validator_set_inclusion_proof =
            parse_rpc_inclusion_proof(next_validator_set_inclusion_proof);

        let now = Instant::now();

        let _proof = prover::proving::prove_validator_set_change(
            previous_proof,
            parse_rpc_block_finality_proof(current_epoch_block_finality, Some(thread_count)),
            next_validator_set_inclusion_proof,
            next_validator_set_storage_data,
        );

        log::info!("Recursive prove time: {}ms", now.elapsed().as_millis());
    }
}
