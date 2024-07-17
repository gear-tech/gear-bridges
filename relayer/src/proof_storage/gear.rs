use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::mpsc::{channel, Receiver, Sender},
};

use futures::executor::block_on;
use gclient::{
    metadata::runtime_types::gear_common::event::DispatchStatus, Event as RuntimeEvent, GearApi,
    GearEvent, WSAddress,
};
use gear_core::ids::{ActorId, MessageId, ProgramId};
use gear_rpc_client::GearApi as WrappedGearApi;
use parity_scale_codec::Encode;
use primitive_types::H256;
use prometheus::Gauge;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::{impl_metered_service, metrics::MeteredService};

use super::{AuthoritySetId, ProofStorage, ProofStorageError};
use prover::proving::{CircuitData, Proof, ProofWithCircuitData};

const CONFIG_FILE_NAME: &str = "config.json";
const UPLOAD_PROGRAM_RETRIES: usize = 16;

pub struct GearProofStorage {
    gear_api: GearApi,
    program: Option<ProgramId>,
    cache: RefCell<Cache>,
    message_channel: Sender<UpdateStateMessage>,
    config_file_path: PathBuf,

    metrics: Metrics,
}

#[derive(Default)]
struct Cache {
    circuit_data: Option<CircuitData>,
    proofs: BTreeMap<u64, Proof>,
}

impl_metered_service! {
    struct Metrics {
        fee_payer_balance: Gauge
    }
}

impl Metrics {
    fn new() -> Self {
        Self::new_inner().expect("Failed to create metrics")
    }

    fn new_inner() -> prometheus::Result<Self> {
        Ok(Self {
            fee_payer_balance: Gauge::new(
                "gear_proof_storage_fee_payer_balance",
                "Gear proof storage fee payer balance",
            )?,
        })
    }
}

impl MeteredService for GearProofStorage {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl ProofStorage for GearProofStorage {
    fn init(
        &mut self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        block_on(self.init_inner(proof_with_circuit_data, genesis_validator_set_id))
    }

    fn get_circuit_data(&self) -> Result<CircuitData, ProofStorageError> {
        block_on(self.get_circuit_data_inner())
    }

    fn get_latest_authority_set_id(&self) -> Option<AuthoritySetId> {
        block_on(self.get_latest_authority_set_id_inner())
    }

    fn get_proof_for_authority_set_id(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        block_on(self.get_proof_for_authority_set_id_inner(authority_set_id))
    }

    fn update(
        &mut self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        block_on(self.update_inner(proof, new_authority_set_id))
    }
}

#[derive(Serialize, Deserialize)]
struct UploadedProgramInfo {
    address: H256,
}

impl GearProofStorage {
    pub async fn new(
        endpoint: &str,
        fee_payer: &str,
        config_folder_path: PathBuf,
    ) -> anyhow::Result<GearProofStorage> {
        let wrapped_gear_api = WrappedGearApi::new(endpoint).await?;

        assert_eq!(
            &endpoint[..5],
            "ws://",
            "Invalid endpoint format: expected ws://..."
        );

        let endpoint: Vec<_> = endpoint[5..].split(':').collect();
        let domain = ["ws://", endpoint[0]].concat();
        let port = endpoint[1].parse::<u16>()?;
        let address = WSAddress::try_new(domain, port)?;

        let gear_api = GearApi::init_with(address, fee_payer).await?;

        let message_channel = run_message_sender(gear_api.clone(), wrapped_gear_api)
            .await
            .expect("Failed to run message sender");

        std::fs::create_dir_all(&config_folder_path)
            .expect("Failed to create directory for gear proof storage config");
        if !config_folder_path.is_dir() {
            panic!("Please provide directory as a path");
        }

        let config_file_path = config_folder_path.join(CONFIG_FILE_NAME);

        let config: Option<UploadedProgramInfo> = std::fs::read_to_string(&config_file_path)
            .ok()
            .map(|ser| serde_json::from_str(&ser).expect("Wrong config file format"));
        let program = config.map(|conf| ActorId::new(conf.address.0));

        let proof_storage = GearProofStorage {
            gear_api,
            cache: Default::default(),
            program,
            message_channel,
            config_file_path,

            metrics: Metrics::new(),
        };

        proof_storage.report_balance_metric().await?;

        Ok(proof_storage)
    }

    async fn init_inner(
        &mut self,
        proof_with_circuit_data: ProofWithCircuitData,
        genesis_validator_set_id: u64,
    ) -> Result<(), ProofStorageError> {
        if self.program.is_some() {
            return Ok(());
        }

        let payload = gear_proof_storage::InitMessage {
            genesis_proof: gear_proof_storage::Proof {
                circuit_data: proof_with_circuit_data.circuit_data.0,
                proof: proof_with_circuit_data.proof.0,
                authority_set_id: genesis_validator_set_id + 1,
            },
        };

        for _ in 0..UPLOAD_PROGRAM_RETRIES {
            log::info!("Uploading proof storage program");

            let res = self.try_upload_program(&payload).await;
            match res {
                Err(err) => {
                    log::error!("Failed to upload proof storage program: {}", err);
                }
                Ok(Some(program)) => {
                    let config = UploadedProgramInfo {
                        address: H256(program.into_bytes()),
                    };
                    let config =
                        serde_json::to_string(&config).expect("Failed to serialize config");

                    std::fs::write(&self.config_file_path, config)
                        .expect("Failed to write config to file");

                    self.program = Some(program);
                    break;
                }
                Ok(None) => {}
            }
        }

        Ok(())
    }

    async fn try_upload_program(
        &self,
        payload: &gear_proof_storage::InitMessage,
    ) -> Result<Option<ActorId>, ProofStorageError> {
        let gas = self
            .gear_api
            .calculate_upload_gas(
                None,
                gear_proof_storage::WASM_BINARY.to_vec(),
                payload.encode(),
                0,
                false,
            )
            .await
            .map_err(Into::<anyhow::Error>::into)?
            .min_limit;

        let mut salt = [0; 32];
        rand::thread_rng().fill_bytes(&mut salt);

        let res = self
            .gear_api
            .upload_program(gear_proof_storage::WASM_BINARY, &salt, &payload, gas, 0)
            .await;

        match res {
            Ok((_, program, _)) => Ok(Some(program)),
            Err(gclient::Error::Module(gclient::errors::ModuleError::Gear(
                gclient::errors::Gear::ProgramAlreadyExists,
            ))) => Ok(None),
            Err(err) => Err(ProofStorageError::InnerError(err.into())),
        }
    }

    async fn get_circuit_data_inner(&self) -> Result<CircuitData, ProofStorageError> {
        if let Some(circuit_data) = self.cache.borrow().circuit_data.as_ref() {
            return Ok(circuit_data.clone());
        }

        let state = self.read_program_state(None).await?;
        let circuit_data = CircuitData::from_bytes(state.latest_proof.circuit_data);

        self.cache.borrow_mut().circuit_data = Some(circuit_data.clone());

        Ok(circuit_data)
    }

    async fn get_latest_authority_set_id_inner(&self) -> Option<AuthoritySetId> {
        let stored_latest = self
            .read_program_state(None)
            .await
            .ok()
            .map(|s| s.latest_proof.authority_set_id);

        let cached = self.cache.borrow().proofs.last_key_value().map(|(&k, _)| k);

        match (stored_latest, cached) {
            (Some(stored), Some(cached)) => Some(stored.max(cached)),
            (Some(stored), None) => Some(stored),
            (None, Some(cached)) => Some(cached),
            (None, None) => None,
        }
    }

    async fn get_proof_for_authority_set_id_inner(
        &self,
        authority_set_id: u64,
    ) -> Result<ProofWithCircuitData, ProofStorageError> {
        let circuit_data = self.get_circuit_data_inner().await?;

        if let Some(proof) = self.cache.borrow().proofs.get(&authority_set_id) {
            return Ok(ProofWithCircuitData {
                circuit_data,
                proof: proof.clone(),
            });
        }

        let state = self.read_program_state(None).await?;
        let Some(&block) = state.proof_blocks.get(&authority_set_id) else {
            return Err(ProofStorageError::NotFound(authority_set_id));
        };

        let block = self
            .gear_api
            .get_block_hash(block)
            .await
            .map_err(Into::<anyhow::Error>::into)?;

        let state = self.read_program_state(Some(block)).await?;
        assert_eq!(state.latest_proof.authority_set_id, authority_set_id);

        let proof = Proof::from_bytes(state.latest_proof.proof);

        let _ = self
            .cache
            .borrow_mut()
            .proofs
            .insert(authority_set_id, proof.clone());

        Ok(ProofWithCircuitData {
            circuit_data,
            proof,
        })
    }

    async fn update_inner(
        &mut self,
        proof: Proof,
        new_authority_set_id: AuthoritySetId,
    ) -> Result<(), ProofStorageError> {
        self.report_balance_metric().await?;

        let _ = self
            .cache
            .borrow_mut()
            .proofs
            .insert(new_authority_set_id, proof.clone());

        let payload = gear_proof_storage::HandleMessage {
            proof: proof.into_bytes(),
            authority_set_id: new_authority_set_id,
        };

        let Some(program) = self.program else {
            return Err(ProofStorageError::NotInitialized);
        };

        let message = UpdateStateMessage {
            payload,
            destination: program,
        };

        self.message_channel
            .send(message)
            .expect("Failed to send message over channel");

        Ok(())
    }

    async fn read_program_state(
        &self,
        at: Option<H256>,
    ) -> Result<gear_proof_storage::State, ProofStorageError> {
        let Some(program) = self.program else {
            return Err(ProofStorageError::NotInitialized);
        };

        let state: gear_proof_storage::State = self
            .gear_api
            .read_state_at(program, vec![], at)
            .await
            .map_err(Into::<anyhow::Error>::into)?;

        Ok(state)
    }

    async fn report_balance_metric(&self) -> anyhow::Result<()> {
        let balance = self
            .gear_api
            .free_balance(self.gear_api.account_id())
            .await?;

        self.metrics.fee_payer_balance.set(balance as f64);

        Ok(())
    }
}

struct UpdateStateMessage {
    payload: gear_proof_storage::HandleMessage,
    destination: ProgramId,
}

enum MessageState {
    Pending {
        message: UpdateStateMessage,
    },
    Submitted {
        message: UpdateStateMessage,
        msg_id: MessageId,
        at_block: u32,
    },
    Failed {
        message: UpdateStateMessage,
        error: anyhow::Error,
    },
}

async fn run_message_sender(
    gear_api: GearApi,
    wrapped_gear_api: WrappedGearApi,
) -> anyhow::Result<Sender<UpdateStateMessage>> {
    let (sender, receiver) = channel();

    tokio::spawn(async move {
        message_sender_inner(&gear_api, &wrapped_gear_api, receiver)
            .await
            .expect("Failed to run message sender");
    });

    Ok(sender)
}

const MESSAGE_RESEND_TIMEOUT: u32 = 100;

async fn message_sender_inner(
    gear_api: &GearApi,
    wrapped_gear_api: &WrappedGearApi,
    receiver: Receiver<UpdateStateMessage>,
) -> anyhow::Result<()> {
    let mut states: Vec<MessageState> = vec![];

    let latest_processed = wrapped_gear_api.latest_finalized_block().await?;
    let mut latest_processed = wrapped_gear_api
        .block_hash_to_number(latest_processed)
        .await?;

    loop {
        for message in receiver.try_iter() {
            states.push(MessageState::Pending { message });
        }

        let mut new_states = vec![];
        for state in states.into_iter() {
            let new_state = match state {
                MessageState::Pending { message } => {
                    let res = submit_message(gear_api, &message).await;

                    match res {
                        Ok((msg_id, block)) => {
                            let block = gear_api.block_number_at(block).await?;

                            MessageState::Submitted {
                                message,
                                msg_id,
                                at_block: block,
                            }
                        }
                        Err(err) => MessageState::Failed {
                            message,
                            error: err,
                        },
                    }
                }
                MessageState::Failed { message, error } => {
                    log::error!("Error sending proof to gear: {}", error);
                    MessageState::Pending { message }
                }
                MessageState::Submitted { .. } => state,
            };

            new_states.push(new_state);
        }
        states = new_states;

        let latest_finalized = wrapped_gear_api.latest_finalized_block().await?;
        let latest_finalized = wrapped_gear_api
            .block_hash_to_number(latest_finalized)
            .await?;

        let mut message_dispatched_events = HashMap::new();
        for block in latest_processed + 1..=latest_finalized {
            let block = wrapped_gear_api.block_number_to_hash(block).await?;
            let events = gear_api.events_at(block).await?;

            for event in events {
                if let RuntimeEvent::Gear(GearEvent::MessagesDispatched { statuses, .. }) = event {
                    for (msg_id, status) in statuses {
                        let msg_id = MessageId::new(msg_id.0);
                        message_dispatched_events.insert(msg_id, status);
                    }
                }
            }
        }
        latest_processed = latest_finalized;

        let mut new_states = vec![];
        for state in states.into_iter() {
            let new_state = match state {
                MessageState::Submitted {
                    message,
                    msg_id,
                    at_block,
                } => match message_dispatched_events.get(&msg_id) {
                    Some(DispatchStatus::Success) => None,
                    Some(DispatchStatus::Failed) => Some(MessageState::Pending { message }),
                    Some(DispatchStatus::NotExecuted) => {
                        log::error!("Message {} at block #{} not executed", msg_id, at_block);
                        None
                    }
                    None => {
                        if at_block + MESSAGE_RESEND_TIMEOUT > latest_finalized {
                            log::warn!(
                                "Timeout for message {} at block #{} exceeded",
                                msg_id,
                                at_block
                            );

                            Some(MessageState::Pending { message })
                        } else {
                            Some(MessageState::Submitted {
                                message,
                                msg_id,
                                at_block,
                            })
                        }
                    }
                },
                _ => Some(state),
            };

            if let Some(new_state) = new_state {
                new_states.push(new_state);
            }
        }
        states = new_states;
    }
}

async fn submit_message(
    gear_api: &GearApi,
    message: &UpdateStateMessage,
) -> anyhow::Result<(MessageId, H256)> {
    let gas = gear_api
        .calculate_handle_gas(
            None,
            message.destination,
            message.payload.encode(),
            0,
            false,
        )
        .await?
        .min_limit;

    Ok(gear_api
        .send_message(message.destination, &message.payload, gas, 0)
        .await?)
}
