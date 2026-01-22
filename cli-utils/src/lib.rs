pub mod common;
pub mod ethereum;
pub mod gear;
pub mod prometheus;

pub use common::*;
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
}
