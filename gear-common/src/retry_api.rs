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
struct ApiHolder<T, R, E>
where
    T: Future<Output = Result<R, E>> + 'static,
    E: ErrorTrait,
{
    future: T,
    api: Pin<Box<gclient::GearApi>>,
}

impl<T, R, E> ApiHolder<T, R, E>
where
    T: Future<Output = Result<R, E>> + 'static,
    E: ErrorTrait,
{
    pub fn new<F>(api: gclient::GearApi, mut retry_future: F) -> Self
    where
        F: FnMut(&'static gclient::GearApi) -> T,
    {
        let api = Box::pin(api);
        let ptr = ptr::from_ref(&api);

        Self {
            future: retry_future(
                unsafe { ptr.as_ref() }
                    .expect("Pointer has been just create from pinned boxed value; qed"),
            ),
            api,
        }
    }

    pub async fn call(self) -> Result<R, E> {
        self.future.await
    }
}

pub async fn retry_n<T, F, R, E>(
    this: &mut Api,
    mut retry_future: F,
    retries_max: usize,
) -> AnyResult<R>
where
    F: FnMut(&'static gclient::GearApi) -> T,
    T: Future<Output = Result<R, E>> + 'static,
    E: ErrorTrait,
{
    let mut retries_timeout = 0;

    loop {
        let api = this.0.gclient();
        let holder = ApiHolder::new(api, &mut retry_future);

        let result = holder.call().await;
        match result {
            Ok(v) => return Ok(v),

            Err(e) => {
                if e.is_timeout() && retries_timeout < retries_max {
                    retries_timeout += 1;
                    // TODO: sleep
                    continue;
                }

                if e.is_transport() {
                    this.0.reconnect().await?;
                    continue;
                }

                return Err(anyhow!("{e:?}"));
            }
        }
    }
}

pub struct Api(ApiProviderConnection);

impl Api {
    pub fn new(connection: ApiProviderConnection) -> Self {
        Self(connection)
    }

    pub async fn retry_n<T, F, R, E>(&mut self, retry_future: F, retries_max: usize) -> AnyResult<R>
    where
        F: FnMut(&'static gclient::GearApi) -> T,
        T: Future<Output = Result<R, E>> + 'static,
        E: ErrorTrait,
    {
        self::retry_n(self, retry_future, retries_max).await
    }
}
