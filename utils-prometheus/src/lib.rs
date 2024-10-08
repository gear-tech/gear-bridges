use axum::{routing::get, Router};
use prometheus::{core::Collector, Encoder, Registry, TextEncoder};
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
