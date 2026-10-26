use ping::PingProgram;
use std::path::PathBuf;

fn main() {
    sails_rs::generate_idl_to_file::<PingProgram>(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ping.idl"),
    )
    .expect("Failed to generate Ping IDL");
}
