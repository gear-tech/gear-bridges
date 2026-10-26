#![no_std]

use ethereum_common::beacon::light::Block as LightBeaconBlock;
const ELECTRA_FRAME: bool = false;

include!("../../../eth-events-common/src/lib-template.rs");
