use anyhow::Context;
use ethereum_client::TxHash;
use gear_rpc_client::{
    dto::{Message, RawBlockInclusionProof},
    GearApi, GearHeader,
};
use gsdk::{config::Header, GearConfig};
use primitive_types::{H256, U256};
use serde::{Deserialize, Serialize};
use sp_consensus_grandpa::GrandpaJustification;
use subxt::{
    blocks::Block,
    config::{substrate::BlakeTwo256, Header as _},
    OnlineClient,
};

pub mod eligible_message_dispatch;
pub mod ethereum;
pub mod gear;

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Debug,
    derive_more::Display,
    Serialize,
    Deserialize,
)]
pub struct AuthoritySetId(pub u64);

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Debug,
    derive_more::Display,
    Serialize,
    Deserialize,
)]
pub struct GearBlockNumber(pub u32);

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Debug,
    derive_more::Display,
    Serialize,
    Deserialize,
)]
pub struct EthereumBlockNumber(pub u64);

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Debug,
    Default,
    derive_more::Display,
    Serialize,
    Deserialize,
)]
pub struct EthereumSlotNumber(pub u64);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageInBlock {
    pub message: Message,
    pub block: GearBlockNumber,
    pub block_hash: H256,
    pub authority_set_id: AuthoritySetId,
}

#[derive(Clone, Copy, Debug)]
pub struct PaidMessage {
    pub nonce: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayedMerkleRoot {
    pub block: GearBlockNumber,
    pub block_hash: H256,
    pub timestamp: u64,
    pub authority_set_id: AuthoritySetId,
    pub merkle_root: H256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EthereumRootInclusion {
    pub block_number: u64,
    pub block_hash: alloy::primitives::B256,
    pub transaction_hash: alloy::primitives::B256,
    pub transaction_index: u64,
    pub log_index: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualifiedMerkleRoot {
    pub root: RelayedMerkleRoot,
    pub inclusion: EthereumRootInclusion,
}

impl std::ops::Deref for QualifiedMerkleRoot {
    type Target = RelayedMerkleRoot;

    fn deref(&self) -> &Self::Target {
        &self.root
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxHashWithSlot {
    pub slot_number: EthereumSlotNumber,
    pub tx_hash: TxHash,
}

#[derive(Clone, Debug)]
pub struct GSdkArgs {
    pub vara_endpoint: String,
}

/// The runtime events consumed by bridge workers, not a partial RuntimeEvent API.
#[derive(Clone, Debug)]
pub enum BridgeEvent {
    MessageQueued(gsdk::gear::gear_eth_bridge::events::MessageQueued),
    QueueMerkleRootChanged(gsdk::gear::gear_eth_bridge::events::QueueMerkleRootChanged),
    AuthoritySetHashChanged(gsdk::gear::gear_eth_bridge::events::AuthoritySetHashChanged),
    QueueOverflowed(gsdk::gear::gear_eth_bridge::events::QueueOverflowed),
    NewAuthorities(gsdk::gear::grandpa::events::NewAuthorities),
    UserMessageSent(gsdk::gear::gear::events::UserMessageSent),
}

impl BridgeEvent {
    fn decode(event: &subxt::events::EventDetails<GearConfig>) -> anyhow::Result<Option<Self>> {
        if let Some(event) =
            event.as_event::<gsdk::gear::gear_eth_bridge::events::MessageQueued>()?
        {
            return Ok(Some(Self::MessageQueued(event)));
        }
        if let Some(event) =
            event.as_event::<gsdk::gear::gear_eth_bridge::events::QueueMerkleRootChanged>()?
        {
            return Ok(Some(Self::QueueMerkleRootChanged(event)));
        }
        if let Some(event) =
            event.as_event::<gsdk::gear::gear_eth_bridge::events::AuthoritySetHashChanged>()?
        {
            return Ok(Some(Self::AuthoritySetHashChanged(event)));
        }
        if let Some(event) =
            event.as_event::<gsdk::gear::gear_eth_bridge::events::QueueOverflowed>()?
        {
            return Ok(Some(Self::QueueOverflowed(event)));
        }
        if let Some(event) = event.as_event::<gsdk::gear::grandpa::events::NewAuthorities>()? {
            return Ok(Some(Self::NewAuthorities(event)));
        }
        if let Some(event) = event.as_event::<gsdk::gear::gear::events::UserMessageSent>()? {
            return Ok(Some(Self::UserMessageSent(event)));
        }
        Ok(None)
    }
}

#[derive(Clone, Debug)]
pub struct GearBlock {
    pub header: Header,
    bridge_events: Vec<BridgeEvent>,
    pub grandpa_justification: Option<GrandpaJustification<GearHeader>>,
}

impl GearBlock {
    pub fn number(&self) -> u32 {
        self.header.number()
    }

    pub fn hash(&self) -> H256 {
        let blake2_hasher = BlakeTwo256;
        self.header.hash_with(blake2_hasher).0.into()
    }

    pub fn bridge_events(&self) -> &[BridgeEvent] {
        &self.bridge_events
    }

    pub fn user_message_sent_events(
        &self,
        from_program: H256,
        to_user: H256,
    ) -> impl Iterator<Item = &[u8]> + use<'_> {
        self.bridge_events
            .iter()
            .filter_map(move |event| match event {
                BridgeEvent::UserMessageSent(gsdk::gear::gear::events::UserMessageSent {
                    message,
                    ..
                }) if message.source().into_bytes() == from_program.0
                    && message.destination().into_bytes() == to_user.0 =>
                {
                    Some(message.payload_bytes())
                }

                _ => None,
            })
    }

    pub async fn from_subxt_block(
        api: &GearApi,
        block: Block<GearConfig, OnlineClient<GearConfig>>,
    ) -> anyhow::Result<Self> {
        let justification = api.get_justification(block.hash()).await?;
        let mut finalized = Self::from_finalized_block(api, block).await?;
        finalized.grandpa_justification = Some(justification);
        Ok(finalized)
    }

    pub async fn from_finalized_block(
        api: &GearApi,
        block: Block<GearConfig, OnlineClient<GearConfig>>,
    ) -> anyhow::Result<Self> {
        let header = block.header().clone();
        let events = api.metadata_events_at(block.hash()).await?;
        let mut bridge_events = Vec::new();
        for event in events.iter() {
            if let Some(event) = BridgeEvent::decode(&event?)? {
                bridge_events.push(event);
            }
        }
        Ok(Self {
            header,
            bridge_events,
            grandpa_justification: None,
        })
    }

    pub async fn from_justification(
        api: &GearApi,
        justification: GrandpaJustification<GearHeader>,
    ) -> anyhow::Result<Self> {
        let block = api
            .api
            .blocks()
            .at(primitive_types::H256::from(
                justification.commit.target_hash.0,
            ))
            .await?;

        let mut finalized = Self::from_finalized_block(api, block).await?;
        finalized.grandpa_justification = Some(justification);
        Ok(finalized)
    }

    /// Produce a raw block inclusion proof from a real GRANDPA justification.
    pub async fn inclusion_proof(&self, api: &GearApi) -> anyhow::Result<RawBlockInclusionProof> {
        let justification = self
            .grandpa_justification
            .clone()
            .context("Merkle-root proof requires a GRANDPA justification")?;
        api.fetch_raw_block_inclusion_proof(self.hash(), Some(justification))
            .await
    }
}

fn message_queued_events_of(
    block: &GearBlock,
) -> impl Iterator<Item = gear_rpc_client::dto::Message> + use<'_> {
    block
        .bridge_events()
        .iter()
        .filter_map(|event| match event {
            BridgeEvent::MessageQueued(gsdk::gear::gear_eth_bridge::events::MessageQueued {
                message,
                ..
            }) => {
                let nonce_be = primitive_types::U256(message.nonce.0).to_big_endian();

                Some(gear_rpc_client::dto::Message {
                    nonce_be,
                    source: message.source.0,
                    destination: message.destination.0,
                    payload: message.payload.clone(),
                })
            }
            _ => None,
        })
}

pub fn message_hash(message: &Message) -> [u8; 32] {
    let data = [
        message.nonce_be.as_ref(),
        message.source.as_ref(),
        message.destination.as_ref(),
        message.payload.as_ref(),
    ]
    .concat();

    let mut hash = [0; 32];
    keccak_hash::keccak_256(&data, &mut hash);

    hash
}

pub mod web_request {
    use super::*;

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct Message {
        pub block: u32,
        pub nonce: U256,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct Messages {
        pub messages: Vec<Message>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct EthTransaction {
        pub tx_hash: String,
    }

    #[derive(Clone, Debug, Default, Deserialize, Serialize)]
    pub struct EthTransactions {
        pub transactions: Vec<EthTransaction>,
    }
}

#[cfg(test)]
mod native_event_tests {
    use super::*;
    use std::str::FromStr;

    #[derive(subxt::ext::scale_decode::DecodeAsType)]
    #[decode_as_type(crate_path = "subxt::ext::scale_decode")]
    struct IncompatibleUserMessageSent {
        message: bool,
    }

    impl subxt::events::StaticEvent for IncompatibleUserMessageSent {
        const PALLET: &'static str = "Gear";
        const EVENT: &'static str = "UserMessageSent";
    }

    #[tokio::test]
    #[ignore = "requires retained native archive and read-only loopback RPC"]
    async fn native_bridge_events_smoke() -> anyhow::Result<()> {
        let rpc = std::env::var("GEAR_ORIGINAL_RPC")?;
        anyhow::ensure!(
            url::Url::parse(&rpc)?.host_str() == Some("127.0.0.1"),
            "loopback RPC required"
        );
        let api = GearApi::new(&rpc, 0).await?;
        assert_eq!(
            format!("{:#x}", api.api.genesis_hash()),
            "0xa12f631344f61644239b70f9ab53e4842e540d8b0c1e7adb9570c06a62c3af3f"
        );
        let archived =
            H256::from_str("0x92ebd056cf9f953ba87d066bc617b33c83d4922dead25f9f0c0068a7b1ddd855")?;
        assert_eq!(api.block_number_to_hash(813).await?, archived);
        let finalized = api.latest_finalized_block().await?;
        let finalized_number = api.block_hash_to_number(finalized).await?;
        assert!(
            finalized_number > 813,
            "current finalized block must follow the archive"
        );
        let mut archived_replies = 0;
        for hash in [archived, finalized] {
            let block =
                GearBlock::from_finalized_block(&api, api.get_block_at(hash).await?).await?;
            assert_eq!(block.hash(), hash);
            assert_eq!(api.block_number_to_hash(block.number()).await?, hash);
            let metadata_events = api.metadata_events_at(hash).await?;
            let mut expected_bridge_events = 0;
            for event in metadata_events.iter() {
                let event = event?;
                expected_bridge_events += usize::from(BridgeEvent::decode(&event)?.is_some());
                if let Some(reply) =
                    event.as_event::<gsdk::gear::gear::events::UserMessageSent>()?
                {
                    let incompatible = event.as_event::<IncompatibleUserMessageSent>();
                    assert!(
                        incompatible.is_err(),
                        "incompatible consumed schema decoded as {:?}",
                        incompatible.ok().flatten().map(|event| event.message)
                    );
                    let source = H256::from(reply.message.source().into_bytes());
                    let destination = H256::from(reply.message.destination().into_bytes());
                    let expected: Vec<_> = block
                        .user_message_sent_events(source, destination)
                        .collect();
                    let actual = api
                        .user_message_sent_events(source, destination, hash)
                        .await?;
                    assert_eq!(
                        actual
                            .iter()
                            .map(|message| message.payload.as_slice())
                            .collect::<Vec<_>>(),
                        expected
                    );
                    assert!(expected.contains(&reply.message.payload_bytes()));
                    let mut wrong_source = source;
                    wrong_source.0[0] ^= 1;
                    assert!(!block
                        .user_message_sent_events(wrong_source, destination)
                        .any(|payload| payload == reply.message.payload_bytes()));
                    archived_replies += usize::from(hash == archived);
                }
            }
            assert_eq!(block.bridge_events().len(), expected_bridge_events);
            let queued = api.message_queued_events(hash).await?;
            let expected_queued: Vec<_> = message_queued_events_of(&block).collect();
            assert_eq!(queued.len(), expected_queued.len());
            for (actual, expected) in queued.iter().zip(expected_queued) {
                assert_eq!(actual.nonce_be, expected.nonce_be);
                assert_eq!(actual.source, expected.source);
                assert_eq!(actual.destination, expected.destination);
                assert_eq!(actual.payload, expected.payload);
            }
            println!(
                "native bridge events block={} hash={hash:#x} total={} consumed={} queued={}",
                block.number(),
                metadata_events.len(),
                block.bridge_events().len(),
                queued.len()
            );
        }
        assert!(archived_replies > 0, "archived constructor reply missing");
        assert_eq!(api.block_number_to_hash(813).await?, archived);
        assert_eq!(api.block_number_to_hash(finalized_number).await?, finalized);
        Ok(())
    }
}
