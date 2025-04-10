use axum::{routing::get, Router};
use log::{Level, Log, Metadata, Record, SetLoggerError};
use prometheus::{
    core::Collector, register_int_counter_vec, Encoder, IntCounterVec, Registry, TextEncoder,
};
use tokio::net::TcpListener;

pub struct MetricsBuilder {
    registry: Registry,
}

pub struct Metrics {
    registry: Registry,
}

pub trait MeteredService {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn Collector>>;
}

#[macro_export]
macro_rules! impl_metered_service {
    (
        $(#[$($struct_attributes:tt)*])*
        $vis:vis struct $struct_name:ident {
            $(
                $(#[$($attributes:tt)*])*
                $field_vis:vis $field_name:ident : $field_type:ty = $constructor:expr
            ),*
            $(,)?
        }
    ) => {
        #[derive(Clone)]
        $(#[$($struct_attributes)*])*
        $vis struct $struct_name {
            $(
                $(#[$($attributes)*])*
                $field_vis $field_name: $field_type
            ),*
        }

        impl $crate::MeteredService for $struct_name {
            fn get_sources(&self) -> impl ::core::iter::IntoIterator<
                Item = ::std::boxed::Box<dyn prometheus::core::Collector>
            > {
                $(
                    let $field_name: ::std::boxed::Box::<dyn prometheus::core::Collector>
                        = ::std::boxed::Box::from(self.$field_name.clone());
                )*

                [
                    $(
                        $field_name
                    ),*
                ]
            }
        }

        impl $struct_name {
            $vis fn new() -> Self {
                let new_inner = || -> prometheus::Result<Self> {
                    Ok(Self {
                        $(
                            $field_name: $constructor ?
                        ),*
                    })
                };

                new_inner().expect("Failed to create metrics")
            }
        }
    }
}

impl MetricsBuilder {
    pub fn new() -> MetricsBuilder {
        MetricsBuilder {
            registry: Registry::new(),
        }
    }

    pub fn register_service(self, service: &impl MeteredService) -> Self {
        for source in service.get_sources() {
            self.registry
                .register(source)
                .expect("Failed to register metric source");
        }
        self
    }

    pub fn build(self) -> Metrics {
        Metrics {
            registry: self.registry,
        }
    }
}

impl Default for MetricsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub async fn run(self, endpoint: String) {
        let reg = self.registry.clone();

        let app = Router::new().route("/metrics", get(move || Self::gather_metrics(reg)));
        let listener = TcpListener::bind(&endpoint)
            .await
            .expect("Failed to create TcpListener");

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
    }

    async fn gather_metrics(registry: Registry) -> String {
        let mut buffer = vec![];

        let encoder = TextEncoder::new();
        let metric_families = registry.gather();
        encoder
            .encode(&metric_families, &mut buffer)
            .expect("Failed to encode metrics");

        String::from_utf8(buffer).expect("Failed to convert metrics to string")
    }
}

use lazy_static::lazy_static;
use prometheus::{opts, register_int_counter, IntCounter};

lazy_static! {
    static ref LOG_ERRORS_TOTAL: IntCounter = register_int_counter!(opts!(
        "log_errors_total",
        "Total number of ERROR level logs recorded."
    ))
    .expect("Failed to register LOG_ERRORS_TOTAL counter");
}

lazy_static! {
    static ref LOG_ERRORS_BY_TARGET_TOTAL: IntCounterVec = register_int_counter_vec!(
        "log_errors_by_target_total",
        "Total number of ERROR level logs recorded, partitioned by log target.",
        &["target"]
    )
    .expect("Failed to register LOG_ERRORS_BY_TARGET_TOTAL counter");
}

pub struct PrometheusErrorCounterLogger<L: Log> {
    inner: L, // The logger that will actually print/write logs
}

impl<L: Log> Log for PrometheusErrorCounterLogger<L> {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Delegate to the underlying logger to decide if a level is enabled
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if record.level() == Level::Error {
            LOG_ERRORS_TOTAL.inc();
            LOG_ERRORS_BY_TARGET_TOTAL
                .with_label_values(&[record.target()])
                .inc();
        }

        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

impl<L: Log + 'static> PrometheusErrorCounterLogger<L> {
    pub fn init(inner: L, max_level: log::LevelFilter) -> Result<(), SetLoggerError> {
        let wrapper = Box::new(PrometheusErrorCounterLogger { inner });
        log::set_boxed_logger(wrapper)?;
        log::set_max_level(max_level); // Set the desired max level
        Ok(())
    }
}
