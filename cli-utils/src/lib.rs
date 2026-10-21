pub mod ethereum;
pub mod gear;
pub mod prometheus;

pub use ethereum::*;
pub use gear::*;
pub use prometheus::*;

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct TestGearCli {
        #[clap(flatten)]
        gear: GearConnectionArgs,
    }

    #[test]
    fn test_gear_endpoint_parsing() {
        let cli = TestGearCli::parse_from(["test", "--gear-endpoint", "wss://example.com"]);
        assert_eq!(cli.gear.gear_endpoint.to_string(), "wss://example.com/");
        assert_eq!(cli.gear.get_endpoint().unwrap(), "wss://example.com/");

        let (host, port) = cli.gear.get_host_port().unwrap();
        assert_eq!(host, "wss://example.com");
        assert_eq!(port, 443);
    }

    #[derive(Parser, Debug)]
    struct TestEthCli {
        #[clap(flatten)]
        eth: EthereumConnectionArgs,
    }

    #[test]
    fn test_eth_parsing() {
        let cli = TestEthCli::parse_from(["test", "--ethereum-endpoint", "https://eth.com"]);
        assert_eq!(cli.eth.ethereum_endpoint, "https://eth.com");
    }

    #[derive(Parser, Debug)]
    struct TestFinalityCli {
        #[clap(flatten)]
        finality: EthereumFinalityArgs,
        #[clap(flatten)]
        identity: EthereumChainIdentityArgs,
    }

    #[test]
    fn ethereum_finality_requires_independent_endpoint_and_approved_identity() {
        let complete = [
            "test", "--ethereum-witness-rpc", "wss://witness.example",
            "--ethereum-chain-id", "560048", "--ethereum-genesis-hash",
            "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ];
        let cli = TestFinalityCli::try_parse_from(complete).unwrap();
        assert_eq!(cli.finality.ethereum_witness_rpc, "wss://witness.example");
        assert_eq!(cli.identity.ethereum_chain_id, 560048);
        for missing in [1, 3, 5] {
            let args: Vec<_> = complete.iter().enumerate()
                .filter_map(|(index, value)| (!(missing..=missing + 1).contains(&index)).then_some(*value))
                .collect();
            assert!(TestFinalityCli::try_parse_from(args).is_err());
        }
    }
}
