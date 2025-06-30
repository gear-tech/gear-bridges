use super::*;

mod block;
mod block_body;
mod block_header;
mod common;
mod execution_payload;
pub mod light;

pub use block::Block;
pub use block_body::BlockBody;
pub use block_header::BlockHeader;
pub use common::*;
pub use execution_payload::ExecutionPayload;

pub mod electra {
    pub use super::{block::electra::Block, block_body::electra::BlockBody, common::electra::*};
}

#[cfg(test)]
mod tests;
