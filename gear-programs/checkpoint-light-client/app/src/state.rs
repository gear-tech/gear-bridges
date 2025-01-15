use checkpoint_light_client_io::{Slot, Keys as SyncCommitteeKeys};
use circular_buffer::CircularBuffer;
use ethereum_common::{network::Network, Hash256, beacon::BlockHeader as BeaconBlockHeader};
use sails_rs::prelude::*;

#[derive(Clone, Debug, Decode, Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
pub enum CheckpointError {
    OutDated,
    NotPresent,
}

pub struct State<const N: usize> {
    pub network: Network,
    pub finalized_header: BeaconBlockHeader,
    pub sync_committee_current: Box<SyncCommitteeKeys>,
    pub sync_committee_next: Box<SyncCommitteeKeys>,
    pub checkpoints: Checkpoints<N>,
    pub replay_back: Option<ReplayBackState>,
}

pub struct ReplayBackState {
    pub finalized_header: BeaconBlockHeader,
    pub sync_committee_next: Option<Box<SyncCommitteeKeys>>,
    pub checkpoints: Vec<(Slot, Hash256)>,
    pub last_header: BeaconBlockHeader,
}

#[derive(Debug, Clone)]
pub struct Checkpoints<const N: usize>(Box<CircularBuffer<N, (Slot, Hash256)>>);

impl<const N: usize> Checkpoints<N> {
    pub fn new() -> Self {
        Self(CircularBuffer::boxed())
    }

    pub fn push(&mut self, slot: Slot, checkpoint: Hash256) {
        self.0.push_back((slot, checkpoint))
    }

    pub fn checkpoints(&self) -> Vec<(Slot, Hash256)> {
        self.0.to_vec()
    }

    pub fn checkpoint(&self, slot: Slot) -> Result<(Slot, Hash256), CheckpointError> {
        let search = |slice: &[(Slot, Hash256)]| match slice
            .binary_search_by(|(slot_current, _checkpoint)| slot_current.cmp(&slot))
        {
            Ok(index) => Ok(slice[index]),
            Err(index_next) => match slice.get(index_next) {
                Some(result) => Ok(*result),
                None => Err(CheckpointError::NotPresent),
            },
        };

        let (left, right) = self.0.as_slices();

        search(left).or(search(right))
    }

    pub fn checkpoint_by_index(&self, index: usize) -> Option<(Slot, Hash256)> {
        self.0.get(index).copied()
    }

    pub fn last(&self) -> Option<(Slot, Hash256)> {
        self.0.back().copied()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &(Slot, Hash256)> {
        self.0.iter()
    }
}

#[test]
fn empty_checkpoints() {
    let checkpoints = Checkpoints::<3>::new();

    assert!(checkpoints.checkpoints().is_empty());
    assert!(matches!(
        checkpoints.checkpoint(0),
        Err(CheckpointError::NotPresent),
    ));
    assert!(matches!(
        checkpoints.checkpoint(1),
        Err(CheckpointError::NotPresent),
    ));
    assert!(matches!(
        checkpoints.checkpoint(u64::MAX),
        Err(CheckpointError::NotPresent),
    ));

    assert!(matches!(checkpoints.checkpoint_by_index(0), None,));
    assert!(matches!(checkpoints.last(), None,));
}

#[cfg(test)]
#[track_caller]
fn compare_checkpoints<const COUNT: usize>(
    data: &[(Slot, Hash256)],
    checkpoints: &Checkpoints<COUNT>,
) {
    for items in data.windows(2) {
        let (slot_start, checkpoint_start) = items[0];
        let (slot_end, checkpoint_end) = items[1];

        let (slot, checkpoint) = checkpoints.checkpoint(slot_start).unwrap();
        assert_eq!(
            slot, slot_start,
            "start; slot = {slot}, {:?}, {data:?}",
            checkpoints.0
        );
        assert_eq!(checkpoint, checkpoint_start);

        let slot_start = slot_start + 1;
        for slot_requested in slot_start..=slot_end {
            let (slot, checkpoint) = checkpoints.checkpoint(slot_requested).unwrap();
            assert_eq!(
                slot, slot_end,
                "slot = {slot}, slot_requested = {slot_requested}, {:?}, {data:?}",
                checkpoints.0
            );
            assert_eq!(checkpoint, checkpoint_end);
        }
    }

    let checkpoints = checkpoints.checkpoints();
    assert_eq!(&checkpoints, data);
}

#[test]
fn checkpoints() {
    use hex_literal::hex;

    const COUNT: usize = 6;

    // Sepolia
    let mut data = vec![
        (
            5_187_904,
            hex!("d902d1b20ad9cc01c963fff6587eb0931729b01ffa2ef93bea152b964186a792").into(),
        ),
        (
            5_187_936,
            hex!("24191bce7807531373065eb296ee15f4658f777ffa887558148ea888efa0feaf").into(),
        ),
        // missing block
        (
            5_187_967,
            hex!("be485bf2d926a79d8354ec003dce82da26b2712e0d25acb888790d0339cc9d58").into(),
        ),
        // missing block
        (
            5_187_999,
            hex!("4e8fa22d42ec2eb962ce110a14669c824891effb283532e154b4e1486a76044b").into(),
        ),
        (
            5_188_032,
            hex!("4401b2d3939a1aa28129400aa5ac4250e1cdec18f1836eb2c2c8c3fc7d49df88").into(),
        ),
        (
            5_188_064,
            hex!("4140f18bf38d656cff5f7944bc83d7bf7d081d4691f9ae78b0280c8b1160f573").into(),
        ),
    ];
    assert_eq!(data.len(), COUNT);

    let mut checkpoints = Checkpoints::<COUNT>::new();

    for (slot, checkpoint) in &data {
        checkpoints.push(*slot, *checkpoint);
    }

    assert!(matches!(
        checkpoints.checkpoint(0),
        Ok(result) if result == data[0],
    ));
    assert!(matches!(
        checkpoints.checkpoint(u64::MAX),
        Err(CheckpointError::NotPresent),
    ));

    compare_checkpoints(&data, &checkpoints);

    // after overwrite data[0] slot = 5_187_936
    data.remove(0);
    data.push((
        5_188_096,
        hex!("5d90dad12f5cebadbc16db005500a19a53618257ceca748d7183cbff45507ca2").into(),
    ));
    checkpoints.push(data.last().unwrap().0, data.last().unwrap().1);

    compare_checkpoints(&data, &checkpoints);

    // after overwrite data[0] slot = 5_187_967
    data.remove(0);
    data.push((
        5_188_128,
        hex!("942b118b30e777151d9040c53471563fe7710df79b57e611bff927c26efa6202").into(),
    ));
    checkpoints.push(data.last().unwrap().0, data.last().unwrap().1);

    compare_checkpoints(&data, &checkpoints);

    // after overwrite data[0] slot = 5_187_967
    data.remove(0);
    data.push((
        5_188_160,
        hex!("4d26e1bfafef3597d6c0cfb67f8c31fd6c7ee970fa855aa9a6bdd8b1670f31cd").into(),
    ));
    checkpoints.push(data.last().unwrap().0, data.last().unwrap().1);

    compare_checkpoints(&data, &checkpoints);

    // after overwrite data[0] slot = 5_187_999
    data.remove(0);
    data.push((
        5_188_192,
        hex!("9c047d8c543183cd407b6955b4bb253bf437b2a4b8cc62859ad46a726f693476").into(),
    ));
    checkpoints.push(data.last().unwrap().0, data.last().unwrap().1);

    compare_checkpoints(&data, &checkpoints);

    // after overwrite data[0] slot = 5_188_032
    data.remove(0);
    data.push((
        5_188_224,
        hex!("b7d7f7efdef892d855640777226bcc7f08e328f264bd879bad61e844c3387f2f").into(),
    ));
    checkpoints.push(data.last().unwrap().0, data.last().unwrap().1);

    compare_checkpoints(&data, &checkpoints);

    // after overwrite data[0] slot = 5_188_064
    data.remove(0);
    data.push((
        5_188_256,
        hex!("7b17d44ed3f5b7ca49aad6069caa7dcf3f496e2b8dee4221dac042c4219894a0").into(),
    ));
    checkpoints.push(data.last().unwrap().0, data.last().unwrap().1);

    compare_checkpoints(&data, &checkpoints);

    // after overwrite data[0] slot = 5_188_096
    data.remove(0);
    data.push((
        5_188_288,
        hex!("10c57533bfcf7343b2003a2ce912958c60805f342455b68c611666fdee1205a5").into(),
    ));
    checkpoints.push(data.last().unwrap().0, data.last().unwrap().1);

    compare_checkpoints(&data, &checkpoints);
}

#[test]
fn checkpoints_with_gaps() {
    use hex_literal::hex;

    const COUNT: usize = 3;

    // Sepolia
    let mut data = vec![
        (
            5_187_904,
            hex!("d902d1b20ad9cc01c963fff6587eb0931729b01ffa2ef93bea152b964186a792").into(),
        ),
        (
            5_187_936,
            hex!("24191bce7807531373065eb296ee15f4658f777ffa887558148ea888efa0feaf").into(),
        ),
        // missing block
        (
            5_187_967,
            hex!("be485bf2d926a79d8354ec003dce82da26b2712e0d25acb888790d0339cc9d58").into(),
        ),
    ];
    assert_eq!(data.len(), COUNT);

    let mut checkpoints = Checkpoints::<COUNT>::new();

    for (slot, checkpoint) in &data {
        checkpoints.push(*slot, *checkpoint);
    }

    // after overwrite data[0] slot = 5_187_936
    data.remove(0);
    data.push((
        5_188_032,
        hex!("4401b2d3939a1aa28129400aa5ac4250e1cdec18f1836eb2c2c8c3fc7d49df88").into(),
    ));
    checkpoints.push(data.last().unwrap().0, data.last().unwrap().1);

    compare_checkpoints(&data, &checkpoints);

    // after overwrite data[0] slot = 5_187_967
    data.remove(0);
    data.push((
        5_188_096,
        hex!("5d90dad12f5cebadbc16db005500a19a53618257ceca748d7183cbff45507ca2").into(),
    ));
    checkpoints.push(data.last().unwrap().0, data.last().unwrap().1);

    compare_checkpoints(&data, &checkpoints);
}

#[test]
fn checkpoints_get() {
    use hex_literal::hex;

    const COUNT: usize = 7;

    // Holesky
    let data = [
        (
            2_498_432,
            hex!("192cbc312720ee203ed023837c7dd7783db6cee1f1b9d57411f348e8a143a308").into(),
        ),
        (
            2_498_464,
            hex!("b89c6d200193f865b85a3f323b75d2b10346564a330229d8a5c695968206faf1").into(),
        ),
        (
            2_498_496,
            hex!("4185e76eb0865e9ae5f8ea7601407261d1db9e66ba10818ebe717976d9bf201c").into(),
        ),
        (
            2_498_527,
            hex!("e722020546e89a17228aa9365e5418aaf09d9c31b014a0b4df911a54702ccd57").into(),
        ),
        (
            2_498_560,
            hex!("b50cd206a8ba4019baad810bbcd4fe1871be4944ea9cb06e15259376e996afde").into(),
        ),
        (
            2_498_592,
            hex!("844300ded738bdad37cc202ad4ade0cc79f0e4aa311e8fee5668cb20341c52aa").into(),
        ),
        (
            2_498_624,
            hex!("aca973372ac65cd5203e1521ba941bbbf836c5e591a9b459ca061c79a5740023").into(),
        ),
    ];
    assert_eq!(data.len(), COUNT);

    let mut checkpoints = Checkpoints::<COUNT>::new();

    for (slot, checkpoint) in &data {
        checkpoints.push(*slot, *checkpoint);
    }

    assert!(checkpoints.checkpoint(2_498_625).is_err());

    for i in 1..data.len() {
        let (slot_previous, _checkpoint) = data[i - 1];
        let (expected_slot, expected_checkpoint) = data[i];
        for slot in (1 + slot_previous)..=expected_slot {
            let (actual_slot, actual_checkpoint) = checkpoints.checkpoint(slot).unwrap();
            assert_eq!(actual_slot, expected_slot, "slot = {slot}");
            assert_eq!(actual_checkpoint, expected_checkpoint);
        }
    }
}
