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
        let cli = TestGearCli::parse_from(&["test", "--gear-endpoint", "wss://example.com"]);
        assert_eq!(cli.gear.endpoint.as_ref().unwrap().to_string(), "wss://example.com/");
        assert_eq!(cli.gear.get_endpoint().unwrap(), "wss://example.com/");
        
        let (host, port) = cli.gear.get_host_port().unwrap();
        assert_eq!(host, "wss://example.com");
        assert_eq!(port, 443);
    }

    #[test]
    fn test_gear_domain_port_fallback() {
        let cli = TestGearCli::parse_from(&["test", "--gear-domain", "ws://localhost", "--gear-port", "9944"]);
        assert!(cli.gear.endpoint.is_none());
        assert_eq!(cli.gear.get_endpoint().unwrap(), "ws://localhost:9944");
        
        let (host, port) = cli.gear.get_host_port().unwrap();
        assert_eq!(host, "ws://localhost");
        assert_eq!(port, 9944);
    }

    #[derive(Parser, Debug)]
    struct TestEthCli {
        #[clap(flatten)]
        eth: EthereumConnectionArgs,
    }

    #[test]
    fn test_eth_parsing() {
        let cli = TestEthCli::parse_from(&["test", "--ethereum-endpoint", "https://eth.com"]);
        assert_eq!(cli.eth.endpoint, "https://eth.com");
    }
}
