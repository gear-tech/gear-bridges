#![no_std]

// Incorporate code generated based on the IDL file
include!(concat!(
    env!("OUT_DIR"),
    "/bridging_payment_vara_supply_client.rs"
));
