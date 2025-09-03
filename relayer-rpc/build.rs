fn main() {
    tonic_prost_build::configure()
        .compile_protos(&["proto/merkle_roots.proto"], &["proto"])
        .unwrap();
}
