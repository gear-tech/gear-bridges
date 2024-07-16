use super::*;

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum StatusStart {
    InProgress,
    Finished,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Status {
    InProcess,
    Finished,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Error {
    AlreadyStarted,
    Verify(sync_update::Error),
    NoFinalityUpdate,
}
