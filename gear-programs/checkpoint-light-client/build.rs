use checkpoint_light_client_io::meta::Metadata;

fn main() {
    gear_wasm_builder::build_with_metadata::<Metadata>()
}
