mod vft_module {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/vft.rs"));
}

pub use vft_module::*;