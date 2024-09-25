mod vft_treasury_module {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/vft-treasury.rs"));
}

pub use vft_treasury_module::*;
