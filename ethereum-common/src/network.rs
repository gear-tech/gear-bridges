use super::*;
use hex_literal::hex;

use Network::*;

#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum Network {
    Mainnet,
    Sepolia,
    Holesky,
    Hoodi,
}

impl Network {
    pub fn genesis_validators_root(&self) -> Hash256 {
        match self {
            Mainnet => hex!("4b363db94e286120d76eb905340fdd4e54bfe9f06bf33ff6cf5ad27f511bfe95"),
            Sepolia => hex!("d8ea171f3c94aea21ebc42a1ed61052acf3f9209c00e4efbaaddac09ed9b8078"),
            Holesky => hex!("9143aa7c615a7f7115e2b6aac319c03529df8242ae705fba9df39b79c59fa8b1"),
            // According to https://github.com/eth-clients/hoodi/blob/2b03cffba84b50759b3476a69334fac8412e217c/metadata/genesis_validators_root.txt
            Hoodi => hex!("212f13fc4df078b6cb7db228f1c8307566dcecf900867401a92023d7ba99cb5f"),
        }
        .into()
    }

    pub fn from_genesis_validators_root(root: [u8; 32]) -> Option<Self> {
        if root == Self::Mainnet.genesis_validators_root().0 {
            return Some(Mainnet);
        }

        if root == Self::Sepolia.genesis_validators_root().0 {
            return Some(Sepolia);
        }

        if root == Self::Holesky.genesis_validators_root().0 {
            return Some(Holesky);
        }

        if root == Self::Hoodi.genesis_validators_root().0 {
            return Some(Hoodi);
        }

        None
    }

    pub fn fork_version(&self, slot: u64) -> [u8; 4] {
        let epoch_electra = self.epoch_electra();
        let epoch_fulu = self.epoch_fulu();
        let epoch = utils::calculate_epoch(slot);
        match self {
            Mainnet => {
                if epoch >= epoch_electra {
                    return hex!("05000000");
                }

                hex!("04000000")
            }

            Sepolia => {
                if epoch >= epoch_fulu {
                    return hex!("90000075");
                }

                if epoch >= epoch_electra {
                    return hex!("90000074");
                }

                hex!("90000073")
            }

            Holesky => {
                if epoch >= epoch_fulu {
                    return hex!("07017000");
                }

                if epoch >= epoch_electra {
                    return hex!("06017000");
                }

                hex!("05017000")
            }

            Hoodi => {
                if epoch >= epoch_fulu {
                    return hex!("70000910");
                }

                if epoch >= epoch_electra {
                    return hex!("60000910");
                }

                hex!("50000910")
            }
        }
    }

    // https://github.com/ethereum/EIPs/blob/55ec2d12e4738585338acdabd1c6400dc7235144/EIPS/eip-7600.md#activation
    pub const fn epoch_electra(&self) -> u64 {
        match self {
            Mainnet => 364_032,
            Sepolia => 222_464,
            Holesky => 115_968,
            Hoodi => 2_048,
        }
    }

    pub const fn epoch_fulu(&self) -> u64 {
        match self {
            Mainnet => todo!(),

            // https://github.com/eth-clients/sepolia/blob/56f0bff41cecab6c661251d72b73ceecc52c5701/metadata/config.yaml#L43
            Sepolia => 272_640,

            // https://github.com/eth-clients/holesky/blob/8aec65f11f0c986d6b76b2eb902420635eb9b815/metadata/config.yaml#L44
            Holesky => 165_120,

            // https://github.com/eth-clients/hoodi/blob/21a110a60e6558a2ba7c819fa4b80029d49ab205/metadata/config.yaml#L43
            Hoodi => 50_688,
        }
    }
}
