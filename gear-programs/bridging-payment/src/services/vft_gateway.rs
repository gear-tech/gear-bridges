mod vft_gateway_module {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/vft-gateway.rs"));
}

pub use vft_gateway_module::*;
