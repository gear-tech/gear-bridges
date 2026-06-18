use axum::{routing::get, Router};
use prometheus::{core::Collector, Encoder, Registry, TextEncoder};
use std::collections::{BTreeMap, HashMap};
use tokio::net::TcpListener;

pub struct MetricsBuilder {
    registries: Vec<Registry>,
}

pub struct Metrics {
    registries: Vec<Registry>,
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
            registries: vec![Registry::new()],
        }
    }

    pub fn register_service(mut self, service: &impl MeteredService) -> Self {
        self.register_service_mut(service);
        self
    }

    pub fn register_service_mut(&mut self, service: &impl MeteredService) -> &mut Self {
        let registry = self
            .registries
            .first()
            .expect("metrics builder always has a default registry");
        register_service(registry, service);

        self
    }

    pub fn register_labeled_service<K, V, I>(
        mut self,
        service: &impl MeteredService,
        labels: I,
    ) -> Self
    where
        K: Into<String>,
        V: Into<String>,
        I: IntoIterator<Item = (K, V)>,
    {
        self.register_labeled_service_mut(service, labels);
        self
    }

    pub fn register_labeled_service_mut<K, V, I>(
        &mut self,
        service: &impl MeteredService,
        labels: I,
    ) -> &mut Self
    where
        K: Into<String>,
        V: Into<String>,
        I: IntoIterator<Item = (K, V)>,
    {
        let labels = labels
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect::<HashMap<_, _>>();
        let registry = Registry::new_custom(None, Some(labels))
            .expect("Failed to create labeled metrics registry");
        register_service(&registry, service);
        self.registries.push(registry);

        self
    }

    pub fn append(&mut self, other: MetricsBuilder) -> &mut Self {
        self.registries.extend(other.registries);
        self
    }

    pub fn build(self) -> Metrics {
        Metrics {
            registries: self.registries,
        }
    }
}

fn register_service(registry: &Registry, service: &impl MeteredService) {
    for source in service.get_sources() {
        registry
            .register(source)
            .expect("Failed to register metric source");
    }
}

impl Default for MetricsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub async fn run(self, endpoint: String) {
        let registries = self.registries.clone();

        let app = Router::new().route(
            "/metrics",
            get(move || Self::gather_metrics(registries.clone())),
        );
        let listener = TcpListener::bind(&endpoint)
            .await
            .expect("Failed to create TcpListener");

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
    }

    async fn gather_metrics(registries: Vec<Registry>) -> String {
        let mut buffer = vec![];

        let encoder = TextEncoder::new();
        let metric_families = Self::gather_metric_families(&registries);
        encoder
            .encode(&metric_families, &mut buffer)
            .expect("Failed to encode metrics");

        String::from_utf8(buffer).expect("Failed to convert metrics to string")
    }

    fn gather_metric_families(registries: &[Registry]) -> Vec<prometheus::proto::MetricFamily> {
        let mut mf_by_name = BTreeMap::new();

        for registry in registries {
            for mut mf in registry.gather() {
                if mf.get_metric().is_empty() {
                    continue;
                }

                let name = mf.get_name().to_owned();
                match mf_by_name.entry(name) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(mf);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        entry.get_mut().mut_metric().extend(mf.take_metric());
                    }
                }
            }
        }

        mf_by_name.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::IntGauge;

    crate::impl_metered_service! {
        struct TestMetrics {
            value: IntGauge = IntGauge::new(
                "duplicate_metric_name",
                "Metric that intentionally appears in more than one labeled registry",
            ),
        }
    }

    #[test]
    fn labeled_registries_allow_duplicate_metric_names() {
        let first = TestMetrics::new();
        first.value.set(1);
        let second = TestMetrics::new();
        second.value.set(2);

        let metrics = MetricsBuilder::new()
            .register_labeled_service(&first, [("relayer", "a")])
            .register_labeled_service(&second, [("relayer", "b")])
            .build();

        let families = Metrics::gather_metric_families(&metrics.registries);

        assert_eq!(families.len(), 1);
        assert_eq!(families[0].get_metric().len(), 2);

        let values = families[0]
            .get_metric()
            .iter()
            .map(|metric| {
                let relayer = metric
                    .get_label()
                    .iter()
                    .find(|label| label.get_name() == "relayer")
                    .expect("relayer label must be present")
                    .get_value()
                    .to_string();
                (relayer, metric.get_gauge().get_value() as i64)
            })
            .collect::<BTreeMap<_, _>>();

        assert_eq!(values.get("a"), Some(&1));
        assert_eq!(values.get("b"), Some(&2));
    }
}
