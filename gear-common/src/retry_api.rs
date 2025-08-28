use std::fmt::Display;

use super::*;

pub trait ErrorTrait: std::fmt::Debug {
    fn is_timeout(&self) -> bool;
    fn is_transport(&self) -> bool;
}

impl ErrorTrait for sails_rs::errors::Error {
    fn is_timeout(&self) -> bool {
        let sails_rs::errors::Error::GClient(e) = self else {
            return false;
        };

        e.is_timeout()
    }

    fn is_transport(&self) -> bool {
        let sails_rs::errors::Error::GClient(e) = self else {
            return false;
        };

        e.is_transport()
    }
}

impl ErrorTrait for GClientError {
    fn is_timeout(&self) -> bool {
        let GClientError::GearSDK(gsdk::Error::Subxt(subxt_error)) = self else {
            return false;
        };

        let subxt::Error::Rpc(subxt::error::RpcError::ClientError(e)) = &**subxt_error else {
            return false;
        };

        let Some(err) = e.downcast_ref::<jsonrpsee_core::ClientError>() else {
            return false;
        };

        matches!(err, jsonrpsee_core::ClientError::RequestTimeout)
    }

    fn is_transport(&self) -> bool {
        let GClientError::GearSDK(gsdk::Error::Subxt(subxt_error)) = self else {
            return false;
        };

        let subxt::Error::Rpc(subxt::error::RpcError::ClientError(e)) = &**subxt_error else {
            return false;
        };

        let Some(err) = e.downcast_ref::<jsonrpsee_core::ClientError>() else {
            return false;
        };

        matches!(
            err,
            jsonrpsee_core::ClientError::RestartNeeded(_)
                | jsonrpsee_core::ClientError::Transport(_)
        )
    }
}

#[allow(dead_code)]
struct ApiHolder<T, R, B: ApiBuilder>
where
    T: Future<Output = AnyResult<R>>,
{
    future: T,
    api: Pin<Box<B::Api>>,
}

impl<T, R, B> ApiHolder<T, R, B>
where
    T: Future<Output = AnyResult<R>>,
    B: ApiBuilder,
{
    pub fn new<'a, F>(api: B::Api, mut retry_future: F) -> Self
    where
        F: FnMut(&'a B::Api) -> T,
    {
        let api = Box::pin(api);
        let ptr = unsafe {
            (&*api as *const <B as ApiBuilder>::Api)
                .as_ref()
                .expect("invalid box")
        };
        Self {
            future: retry_future(ptr),
            api,
        }
    }

    pub async fn call(self) -> AnyResult<R> {
        self.future.await
    }
}

pub async fn retry_n<'a, T, F, R, B>(
    this: &'a mut Api,
    mut retry_future: F,
    retries_max: usize,
    builder: B,
) -> AnyResult<R>
where
    B: ApiBuilder,
    F: FnMut(&'a B::Api) -> T,
    T: Future<Output = AnyResult<R>> + 'a,
{
    let mut retries_timeout = 0;

    loop {
        let api = match builder.build(&mut this.0) {
            Ok(api) => api,
            Err(e) => {
                if e.is_timeout() && retries_timeout < retries_max {
                    retries_timeout += 1;
                    continue;
                }

                if e.is_transport() {
                    this.0.reconnect().await?;
                    continue;
                }

                return Err(anyhow!("{e:?}"));
            }
        };
        let holder = ApiHolder::<_, _, B>::new(api, &mut retry_future);

        let result = holder.call().await;
        match result {
            Ok(v) => return Ok(v),

            Err(e) => {
                if let Some(e) = e.downcast_ref::<B::Error>() {
                    if e.is_timeout() && retries_timeout < retries_max {
                        retries_timeout += 1;
                        // TODO: sleep
                        continue;
                    }

                    if e.is_transport() {
                        this.0.reconnect().await?;
                        continue;
                    }
                }

                return Err(e);
            }
        }
    }
}

pub struct Api(ApiProviderConnection);

impl Api {
    pub fn new(connection: ApiProviderConnection) -> Self {
        Self(connection)
    }

    pub async fn retry_n<'a, T, F, R, B: ApiBuilder>(
        &'a mut self,
        retry_future: F,
        retries_max: usize,
        builder: B,
    ) -> AnyResult<R>
    where
        F: FnMut(&'a B::Api) -> T,
        T: 'a + Future<Output = AnyResult<R>>,
    {
        self::retry_n(self, retry_future, retries_max, builder).await
    }
}

pub trait ApiBuilder {
    type Error: ErrorTrait + Send + Sync + Display + 'static;
    type Api: 'static;

    fn build(&self, conn: &mut ApiProviderConnection) -> Result<Self::Api, Self::Error>;
}

pub struct GearApiBuilder;

impl ApiBuilder for GearApiBuilder {
    type Error = GClientError;
    type Api = GearApi;

    fn build(&self, conn: &mut ApiProviderConnection) -> Result<Self::Api, Self::Error> {
        Ok(conn.client())
    }
}

pub struct GClientApiBuilder;

impl ApiBuilder for GClientApiBuilder {
    type Error = GClientError;
    type Api = gclient::GearApi;

    fn build(&self, conn: &mut ApiProviderConnection) -> Result<Self::Api, Self::Error> {
        Ok(conn.gclient())
    }
}

pub struct GClientWithSuriBuilder<'a>(pub &'a str);

impl<'a> ApiBuilder for GClientWithSuriBuilder<'a> {
    type Error = GClientError;
    type Api = gclient::GearApi;

    fn build(&self, conn: &mut ApiProviderConnection) -> Result<Self::Api, Self::Error> {
        gclient::GearApi::from(conn.client().api).with(self.0)
    }
}

pub struct RemotingBuilder<'a>(pub &'a str);

impl<'a> ApiBuilder for RemotingBuilder<'a> {
    type Error = GClientError;
    type Api = GClientRemoting;

    fn build(&self, conn: &mut ApiProviderConnection) -> Result<Self::Api, Self::Error> {
        let with_suri = GClientWithSuriBuilder(self.0).build(conn)?;

        Ok(GClientRemoting::new(with_suri))
    }
}
