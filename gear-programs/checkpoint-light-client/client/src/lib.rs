#![no_std]

// Incorporate code generated based on the IDL file
include!(concat!(
    env!("OUT_DIR"),
    "/checkpoint_light_client_client.rs"
));
