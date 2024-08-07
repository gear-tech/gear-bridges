use super::*;
use circular_buffer::CircularBuffer;
use io::{
    ethereum_common::{network::Network, Hash256, SLOTS_PER_EPOCH},
    BeaconBlockHeader, CheckpointError, Slot, SyncCommitteeKeys,
};

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
pub struct Checkpoints<const N: usize> {
    checkpoints: Box<CircularBuffer<N, Hash256>>,
    slots: Vec<(usize, Slot)>,
}

impl<const N: usize> Checkpoints<N> {
    pub fn new() -> Self {
        Self {
            checkpoints: CircularBuffer::boxed(),
            slots: Vec::with_capacity(N / 2),
        }
    }

    pub fn push(&mut self, slot: Slot, checkpoint: Hash256) {
        let len = self.checkpoints.len();
        let overwrite = len >= self.checkpoints.capacity();
        let slot_last = self.last().map(|(slot, _checkpoint)| slot);

        self.checkpoints.push_back(checkpoint);

        if overwrite {
            let maybe_index_second = self.slots.get(1).map(|(index, _slot)| *index);
            match (self.slots.get_mut(0), maybe_index_second) {
                (Some((index_first, slot_first)), Some(index_second)) => {
                    if *index_first == 0 && index_second == 1 {
                        self.slots.remove(0);
                        self.slots[0].0 -= 1;
                    } else {
                        *slot_first += SLOTS_PER_EPOCH;
                    }
                }

                (Some((_index_first, slot_first)), None) => *slot_first += SLOTS_PER_EPOCH,

                _ => unreachable!(),
            }

            // adjust indexes. We skip the first item since it always points to the first checkpoint.
            for (index, _) in self.slots.iter_mut().skip(1) {
                *index -= 1;
            }
        }

        match self.slots.last() {
            None => (),

            Some((_, slot_previous))
                if slot % SLOTS_PER_EPOCH != 0
                    || slot_last
                        .map(|slot_last| slot_last + SLOTS_PER_EPOCH < slot)
                        .unwrap_or(false)
                    || slot_previous % SLOTS_PER_EPOCH != 0 => {}

            _ => return,
        }

        self.slots
            .push((if overwrite { len - 1 } else { len }, slot));
    }

    pub fn checkpoints(&self) -> Vec<(Slot, Hash256)> {
        let mut result = Vec::with_capacity(self.checkpoints.len());
        for indexes in self.slots.windows(2) {
            let (index_first, slot_first) = indexes[0];
            let (index_second, _slot_second) = indexes[1];
            if index_first + 1 == index_second {
                result.push((slot_first, self.checkpoints[index_first]));
            } else {
                result.extend(
                    self.checkpoints
                        .iter()
                        .skip(index_first)
                        .take(index_second - index_first)
                        .enumerate()
                        .map(|(slot, checkpoint)| {
                            (slot_first + SLOTS_PER_EPOCH * slot as u64, *checkpoint)
                        }),
                );
            }
        }

        if let Some((index_first, slot_first)) = self.slots.last() {
            result.extend(self.checkpoints.iter().skip(*index_first).enumerate().map(
                |(slot, checkpoint)| (*slot_first + SLOTS_PER_EPOCH * slot as u64, *checkpoint),
            ));
        }

        result
    }

    pub fn checkpoint(&self, slot: Slot) -> Result<(Slot, Hash256), CheckpointError> {
        let Some((index_last, slot_last)) = self.slots.last() else {
            return Err(CheckpointError::NotPresent);
        };

        match self
            .slots
            .binary_search_by(|(_index, slot_checkpoint)| slot_checkpoint.cmp(&slot))
        {
            Ok(index) => Ok((slot, self.checkpoints[self.slots[index].0])),

            Err(0) => Err(CheckpointError::OutDated),

            Err(index) if index < self.slots.len() => {
                let (index_previous, slot_previous) = self.slots[index - 1];
                let (index_next, slot_next) = self.slots[index];

                let gap = match (slot_next - slot_previous) % SLOTS_PER_EPOCH {
                    // both slots are divisable by SLOTS_PER_EPOCH and the distance
                    // between them is greater than SLOTS_PER_EPOCH
                    0 if slot_previous + SLOTS_PER_EPOCH < slot_next => true,
                    _ => false,
                };

                let offset = ((slot - 1 - slot_previous) / SLOTS_PER_EPOCH + 1) as usize;
                let slot_checkpoint = slot_previous + offset as u64 * SLOTS_PER_EPOCH;
                if slot_previous % SLOTS_PER_EPOCH != 0
                    || slot_next < slot_checkpoint
                    || slot_next > slot_checkpoint + SLOTS_PER_EPOCH
                    || gap
                {
                    Ok((slot_next, self.checkpoints[index_next]))
                } else {
                    Ok((slot_checkpoint, self.checkpoints[index_previous + offset]))
                }
            }

            _ => {
                let offset = ((slot - 1 - slot_last) / SLOTS_PER_EPOCH + 1) as usize;
                let slot_checkpoint = slot_last + offset as u64 * SLOTS_PER_EPOCH;
                let index = index_last + offset;
                match self.checkpoints.get(index) {
                    Some(checkpoint) => Ok((slot_checkpoint, *checkpoint)),
                    None => Err(CheckpointError::NotPresent),
                }
            }
        }
    }

    pub fn checkpoint_by_index(&self, index: usize) -> Option<(Slot, Hash256)> {
        match self
            .slots
            .binary_search_by(|(index_data, _slot)| index_data.cmp(&index))
        {
            Ok(index) => {
                let (index_checkpoint, slot) = self.slots[index];

                Some((slot, self.checkpoints[index_checkpoint]))
            }

            Err(0) => None,

            Err(index_data) => {
                let checkpoint = self.checkpoints.get(index)?;

                let (index_start, slot_start) = self.slots[index_data - 1];
                let slot = slot_start + (index - index_start) as u64 * SLOTS_PER_EPOCH;

                Some((slot, *checkpoint))
            }
        }
    }

    pub fn last(&self) -> Option<(Slot, Hash256)> {
        match self.checkpoints.len() {
            0 => None,
            len => self.checkpoint_by_index(len - 1),
        }
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
            "start; slot = {slot}, {:?}, {:?}, {data:?}",
            checkpoints.slots, checkpoints.checkpoints
        );
        assert_eq!(checkpoint, checkpoint_start);

        let slot_start = slot_start + 1;
        for slot_requested in slot_start..=slot_end {
            let (slot, checkpoint) = checkpoints.checkpoint(slot_requested).unwrap();
            assert_eq!(
                slot, slot_end,
                "slot = {slot}, slot_requested = {slot_requested}, {:?}, {:?}, {data:?}",
                checkpoints.slots, checkpoints.checkpoints
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
        Err(CheckpointError::OutDated),
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
