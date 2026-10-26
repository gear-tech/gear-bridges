#![no_std]
use ethereum_common::beacon::light::electra::Block as LightBeaconBlock;
const ELECTRA_FRAME: bool = true;

include!("../../../eth-events-common/src/lib-template.rs");
