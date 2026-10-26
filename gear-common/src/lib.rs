pub mod api_provider;
pub mod durable;
pub mod native_wrapper;
pub mod receipt_framing;
pub mod signer_ownership;

// The constant is intentionally duplicated since vara-runtime is too heavy dependency.
pub const UNITS: u128 = 1_000_000_000_000;
