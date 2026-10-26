pub mod ethereum;
pub mod gear;
pub mod prometheus;

pub use ethereum::*;
pub use gear::*;
pub use prometheus::*;

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

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
    struct TestFinalityCli {
        #[clap(flatten)]
        finality: EthereumFinalityArgs,
        #[clap(flatten)]
        identity: EthereumChainIdentityArgs,
    }

    #[test]
    fn ethereum_authority_requires_beacon_approval_without_a_data_witness() {
        let complete = [
            "test",
            "--ethereum-finality-beacon-rpc",
            "https://beacon.example",
            "--ethereum-finality-bootstrap",
            "approved-bootstrap.json",
            "--ethereum-finality-bootstrap-sha256",
            "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123",
            "--ethereum-finality-store",
            "owned-evidence",
            "--ethereum-chain-id",
            "560048",
            "--ethereum-genesis-hash",
            "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123",
        ];
        let parse = |args: Vec<&str>| {
            TestFinalityCli::command()
                .mut_args(|arg| arg.env(None::<&str>))
                .try_get_matches_from(args)
        };
        assert!(parse(complete.to_vec()).is_ok());
        for missing in (1..complete.len()).step_by(2) {
            let args = complete
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    (!(missing..=missing + 1).contains(&index)).then_some(*value)
                })
                .collect();
            assert!(parse(args).is_err());
        }
    }
}
