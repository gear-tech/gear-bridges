#![no_std]

// Incorporate code generated based on the IDL file
include!(concat!(env!("OUT_DIR"), "/historical_proxy_client.rs"));
