use super::*;
use hex_literal::hex;

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Network {
    Mainnet,
    Sepolia,
    Holesky,
}

impl Network {
    pub fn genesis_validators_root(&self) -> Hash256 {
        use Network::*;

        match self {
            Mainnet => hex!("4b363db94e286120d76eb905340fdd4e54bfe9f06bf33ff6cf5ad27f511bfe95"),
            Sepolia => hex!("d8ea171f3c94aea21ebc42a1ed61052acf3f9209c00e4efbaaddac09ed9b8078"),
            Holesky => hex!("9143aa7c615a7f7115e2b6aac319c03529df8242ae705fba9df39b79c59fa8b1"),
        }.into()
    }
}
