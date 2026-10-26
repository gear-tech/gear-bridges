use super::*;
use serde_json::Value;

fn run(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap();
}
fn init(bootstrap: utils::Bootstrap, update: utils::Update) -> Init {
    let (update, aggregate) = update_wire(update).unwrap();
    Init {
        network: Network::Hoodi,
        trusted_bootstrap_root: bootstrap.header.tree_hash_root(),
        bootstrap_header: bootstrap.header,
        sync_committee_current_pub_keys: keys(&bootstrap.current_sync_committee).unwrap(),
        sync_committee_current_aggregate_pubkey: bootstrap.current_sync_committee.aggregate_pubkey,
        sync_committee_current_branch: bootstrap
            .current_sync_committee_branch
            .into_iter()
            .map(|value| value.0 .0)
            .collect(),
        update,
        sync_aggregate_encoded: aggregate,
    }
}
fn approval(init: &Init) -> BeaconFinalityConfig {
    let at = (Network::Hoodi.genesis_time() + init.update.signature_slot * 12 + 1) * 1000;
    let value = BootstrapApproval {
        schema_version: 1,
        chain_id: 560048,
        genesis_hash: B256::repeat_byte(1),
        network: Network::Hoodi,
        genesis_validators_root: B256::from(Network::Hoodi.genesis_validators_root().0),
        genesis_time: Network::Hoodi.genesis_time(),
        trusted_bootstrap_root: B256::from(init.trusted_bootstrap_root.0),
        trusted_bootstrap_slot: init.bootstrap_header.slot,
        approved_at_ms: at,
        bootstrap_expires_at_ms: at + 1000,
    };
    let bytes = serde_json::to_vec(&value).unwrap();
    BeaconFinalityConfig::parse(bytes.clone(), digest(&bytes), "http://127.0.0.1:1").unwrap()
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "native-beacon-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn retain(directory: &Path, config: &BeaconFinalityConfig, init: Init) -> B256 {
    let record = ProofRecord::Bootstrap {
        approval_hash: config.approval_hash.0,
        verified_at_ms: config.approval.approved_at_ms,
        init,
    };
    let bytes = Bytes::from(record.encode());
    let hash = digest(&bytes);
    gear_common::durable::immutable_json(&directory.join("approval.json"), &config.approval_bytes)
        .unwrap();
    gear_common::durable::immutable_json(&directory.join(format!("proof-{hash:x}.json")), &bytes)
        .unwrap();
    gear_common::durable::immutable_json(&directory.join("revision-0.json"), &hash).unwrap();
    hash
}
fn hoodi() -> Init {
    let bootstrap: utils::BootstrapResponse = serde_json::from_slice(include_bytes!(
        "../../../tests/src/checkpoint_light_client/chain-data/hoodi-bootstrap-489.json"
    ))
    .unwrap();
    let updates: utils::UpdateResponse = serde_json::from_slice(include_bytes!(
        "../../../tests/src/checkpoint_light_client/chain-data/hoodi-update-489.json"
    ))
    .unwrap();
    init(bootstrap.data, updates.into_iter().next().unwrap().data)
}

#[test]
fn strict_approval_and_native_bootstrap_reject_mutations() {
    run(|| {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let init = hoodi();
            let config = approval(&init);
            let at = config.approval.approved_at_ms;
            let verified =
                BeaconAuthority::bootstrap_record(&config, init.clone(), at, B256::repeat_byte(2))
                    .await
                    .unwrap();
            assert_eq!(verified.finalized_header, init.update.finalized_header);
            for mutation in 0..6 {
                let mut invalid = init.clone();
                match mutation {
                    0 => invalid.trusted_bootstrap_root.0[0] ^= 1,
                    1 => invalid.sync_aggregate_encoded.push(0),
                    2 => invalid.update.finality_branch[0][0] ^= 1,
                    3 => invalid.update.sync_committee_next_branch.as_mut().unwrap()[0][0] ^= 1,
                    4 => invalid.update.attested_header.body_root.0[0] ^= 1,
                    _ => invalid.network = Network::Mainnet,
                }
                assert!(
                    BeaconAuthority::bootstrap_record(&config, invalid, at, B256::repeat_byte(2))
                        .await
                        .is_err(),
                    "mutation {mutation}"
                );
            }
            assert!(
                BeaconAuthority::bootstrap_record(&config, init.clone(), at - 1, B256::ZERO)
                    .await
                    .is_err()
            );
            assert!(BeaconAuthority::bootstrap_record(
                &config,
                init,
                config.approval.bootstrap_expires_at_ms + 1,
                B256::ZERO
            )
            .await
            .is_err());
            let bytes = config.approval_bytes.to_vec();
            assert!(
                BeaconFinalityConfig::parse(bytes.clone(), B256::ZERO, "https://example.org")
                    .is_err()
            );
            for url in [
                "http://example.org",
                "http://localhost:5052",
                "file:///tmp/beacon",
                "ws://127.0.0.1",
            ] {
                assert!(
                    BeaconFinalityConfig::parse(bytes.clone(), config.approval_hash, url).is_err()
                );
            }
            for field in ["schemaVersion", "genesisTime", "chainId", "unknown"] {
                let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                value[field] = Value::from(42);
                let mutated = serde_json::to_vec(&value).unwrap();
                assert!(BeaconFinalityConfig::parse(
                    mutated.clone(),
                    digest(&mutated),
                    "https://example.org"
                )
                .is_err());
            }
            let mut bad_keys: utils::BootstrapResponse = serde_json::from_slice(include_bytes!(
                "../../../tests/src/checkpoint_light_client/chain-data/hoodi-bootstrap-489.json"
            ))
            .unwrap();
            bad_keys.data.current_sync_committee.pubkeys.0[0].0 .0 = [0; 48];
            assert!(keys(&bad_keys.data.current_sync_committee).is_err());
        })
    });
}

#[test]
fn genuine_signed_transcript_reloads_after_approval_expiry_and_rejects_cache_tampering() {
    run(|| {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let init = hoodi();
            let config = approval(&init);
            assert!(now_ms().unwrap() > config.approval.bootstrap_expires_at_ms);
            let directory = Directory::new();
            retain(&directory.0, &config, init);
            let authority = BeaconAuthority::open(config.clone(), &directory.0)
                .await
                .unwrap();
            assert!(
                BeaconAuthority::open(config.clone(), &directory.0)
                    .await
                    .is_err(),
                "exclusive owner lock"
            );
            let state = authority.state.lock().await.clone();
            assert_eq!(
                authority
                    .replay(B256::from(state.transition))
                    .await
                    .unwrap()
                    .encode(),
                state.encode()
            );
            drop(authority);
            let authority = BeaconAuthority::open(config.clone(), &directory.0)
                .await
                .unwrap();
            let mut invalid = authority.state.lock().await.as_ref().clone();
            invalid.finalized_header.state_root.0[0] ^= 1;
            drop(authority);
            super::super::finality_archive::atomic_json(
                &directory.0.join("store.json"),
                &Bytes::from(invalid.encode()),
            )
            .unwrap();
            assert!(BeaconAuthority::open(config.clone(), &directory.0)
                .await
                .is_err());
            let fresh = Directory::new();
            assert!(
                BeaconAuthority::open(config, &fresh.0).await.is_err(),
                "expired approval cannot fresh-bootstrap"
            );
        })
    });
}

#[test]
fn signed_fulu_rollover_cas_and_missing_transition_hold() {
    run(|| {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
        // Same publicly labelled deterministic signed vectors exercised by actual actor gtest.
        let data: Value = serde_json::from_slice(include_bytes!("../../../tests/src/checkpoint_light_client/chain-data/synthetic-fulu-transitions.json")).unwrap();
        let bootstrap: utils::BootstrapResponse = serde::Deserialize::deserialize(&data["bootstrap"]).unwrap();
        let update: utils::Update = serde::Deserialize::deserialize(&data["updates"][0]["data"]).unwrap();
        let init = init(bootstrap.data, update);
        let config = approval(&init);
        let directory = Directory::new();
        retain(&directory.0, &config, init);
        let authority = BeaconAuthority::open(config.clone(), &directory.0).await.unwrap();
        let base = authority.state.lock().await.clone();
        let update: utils::Update = serde::Deserialize::deserialize(&data["updates"][3]["data"]).unwrap();
        let (update, aggregate) = update_wire(update).unwrap();
        authority.commit_update(base.clone(), update.clone(), aggregate.clone()).await.unwrap();
        assert!(authority.commit_update(base, update, aggregate).await.is_err(), "unchanged revision CAS");
        let base = authority.state.lock().await.clone();
        let update: utils::Update = serde::Deserialize::deserialize(&data["updates"][5]["data"]).unwrap();
        let (update, aggregate) = update_wire(update).unwrap();
        authority.commit_update(base, update, aggregate).await.unwrap();
        let next = authority.state.lock().await.clone();
        assert_eq!(next.revision, 2);
        drop(authority);
        let reloaded = BeaconAuthority::open(config.clone(), &directory.0).await.unwrap();
        assert_eq!(reloaded.state.lock().await.encode(), next.encode());
        drop(reloaded);
        fs::remove_file(directory.0.join("revision-1.json")).unwrap();
        assert!(BeaconAuthority::open(config, &directory.0).await.is_err(), "missing history cannot trust cached keys");
    })
    });
}

#[test]
fn strict_versioned_checkpoint_and_all_legacy_fences() {
    let common = alloy::consensus::Header {
        number: 10,
        ..Default::default()
    };
    let primary = alloy::consensus::Header {
        number: 12,
        ..Default::default()
    };
    let witness = alloy::consensus::Header {
        number: 11,
        ..Default::default()
    };
    let legacy = FinalityCheckpoint::Legacy(LegacyFinalityCheckpoint {
        schema_version: 1,
        policy: "independent-two-el-v1".into(),
        chain_id: 1,
        genesis_hash: B256::repeat_byte(1),
        common_header: common,
        primary_finalized_header: primary,
        witness_finalized_header: witness,
    });
    let bytes = serde_json::to_vec(&legacy).unwrap();
    assert_eq!(
        serde_json::to_vec(&serde_json::from_slice::<FinalityCheckpoint>(&bytes).unwrap()).unwrap(),
        bytes
    );
    assert_eq!(legacy.fences().unwrap().len(), 3);
    for field in ["schema_version", "policy", "beacon", "unknown"] {
        let mut value = serde_json::to_value(&legacy).unwrap();
        value[field] = Value::from(2);
        assert!(serde_json::from_value::<FinalityCheckpoint>(value).is_err());
    }
}

#[test]
fn signed_loopback_full_block_rlp_binding_and_proof_reload() {
    run(|| {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let mut execution: Header = Header::default();
            execution.inner.number = 12;
            execution.inner.requests_hash = Some(B256::repeat_byte(3));
            execution.hash = execution.inner.hash_slow();
            let head = execution.clone();
            let fixture = test_fixtures::LocalBeaconFixture::new(
                31337,
                B256::repeat_byte(7),
                Arc::new(move || head.clone()),
            )
            .await
            .unwrap();
            let authority = BeaconAuthority::open(fixture.config.clone(), &fixture.directory)
                .await
                .unwrap();
            let state = authority.refresh().await.unwrap();
            let block = FullBlock::Electra(
                authority
                    .client
                    .get_block_by_hash(&state.finalized_header.tree_hash_root().0)
                    .await
                    .unwrap(),
            );
            assert_eq!(
                block
                    .binding(&fixture.config.approval.network, &state.finalized_header)
                    .unwrap(),
                (execution.number, execution.hash)
            );
            let proof = BeaconCheckpointProof {
                genesis_validators_root: fixture.config.approval.genesis_validators_root,
                bootstrap_approval_hash: fixture.config.approval_hash,
                finalized_beacon_header_scale: state.finalized_header.encode().into(),
                proof_digest: authority
                    .record(&ProofRecord::FullBlock {
                        transition: state.transition,
                        block,
                    })
                    .unwrap(),
            };
            authority
                .verify_proof(&proof, &execution.inner)
                .await
                .unwrap();
            drop(authority);
            let reloaded = BeaconAuthority::open(fixture.config.clone(), &fixture.directory)
                .await
                .unwrap();
            reloaded
                .verify_proof(&proof, &execution.inner)
                .await
                .unwrap();
            let mut altered = execution.inner.clone();
            altered.requests_hash = Some(B256::repeat_byte(4));
            assert!(reloaded.verify_proof(&proof, &altered).await.is_err());
            let mut wrong_proof = proof.clone();
            wrong_proof.proof_digest.0[0] ^= 1;
            assert!(reloaded
                .verify_proof(&wrong_proof, &execution.inner)
                .await
                .is_err());
            let peer_directory = fixture.directory.join("peer");
            fs::create_dir(&peer_directory).unwrap();
            let peer_config = fixture
                .config
                .clone()
                .with_proof_sources(vec![fixture.directory.clone()])
                .unwrap();
            let peer = BeaconAuthority::open(peer_config, &peer_directory)
                .await
                .unwrap();
            // The source owner is still active. Only immutable linked proof records
            // are read: deliberately corrupt its mutable cache and revision index.
            fs::write(fixture.directory.join("store.json"), b"untrusted cache").unwrap();
            fs::write(
                fixture.directory.join("revision-0.json"),
                b"untrusted revision",
            )
            .unwrap();
            peer.verify_proof(&proof, &execution.inner).await.unwrap();
            assert!(
                !peer.record_path(proof.proof_digest).exists(),
                "read-only imports must not copy peer history"
            );
            fs::remove_file(reloaded.record_path(proof.proof_digest)).unwrap();
            assert!(reloaded
                .verify_proof(&proof, &execution.inner)
                .await
                .is_err());
            assert!(peer.verify_proof(&proof, &execution.inner).await.is_err());
        })
    });
}
