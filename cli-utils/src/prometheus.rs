use clap::Args;

#[derive(Args, Clone, Debug)]
pub struct PrometheusArgs {
    /// Address of the prometheus endpoint
    #[arg(
        long = "prometheus-endpoint",
        default_value = "0.0.0.0:9090",
        env = "PROMETHEUS_ENDPOINT"
    )]
    pub endpoint: String,
}
