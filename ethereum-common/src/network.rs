use super::*;
use hex_literal::hex;

use Network::*;

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Network {
    Mainnet,
    Sepolia,
    Holesky,
}

impl Network {
    pub fn genesis_validators_root(&self) -> Hash256 {
        match self {
            Mainnet => hex!("4b363db94e286120d76eb905340fdd4e54bfe9f06bf33ff6cf5ad27f511bfe95"),
            Sepolia => hex!("d8ea171f3c94aea21ebc42a1ed61052acf3f9209c00e4efbaaddac09ed9b8078"),
            Holesky => hex!("9143aa7c615a7f7115e2b6aac319c03529df8242ae705fba9df39b79c59fa8b1"),
        }
        .into()
    }

    pub fn fork_version(&self, slot: u64) -> [u8; 4] {
        let epoch_electra = self.epoch_electra();
        let epoch = utils::calculate_epoch(slot);
        match self {
            Mainnet => hex!("04000000"),
            Sepolia => hex!("90000073"),

            Holesky => {
                if epoch >= epoch_electra {
                    return hex!("06017000");
                }

                hex!("05017000")
            }
        }
    }

    // https://github.com/ethereum/EIPs/blob/e7d6d3a75b646bdcf6b957623c92c10e749163ce/EIPS/eip-7600.md#activation
    pub const fn epoch_electra(&self) -> u64 {
        match self {
            Holesky => 115_968,
            Sepolia => 222_464,
            _ => todo!(),
        }
    }
}
