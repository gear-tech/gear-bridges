use super::*;

pub struct State {
    pub genesis: Genesis,
    pub finalized_header: BeaconBlockHeader,
    pub sync_committee_current: Vec<G1>,
    pub sync_committee_next: Option<Vec<G1>>,
}
