use async_trait::async_trait;
use ethers::providers::{FromErr, Middleware, PendingTransaction};
use ethers::types::{transaction::eip2718::TypedTransaction, *};
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug)]
/// Middleware used for calculating nonces locally, useful for signing multiple
/// consecutive transactions without waiting for them to hit the mempool.
pub struct LockedNonceManagerMiddleware<M> {
    inner: M,
    initialized: AtomicBool,
    nonce: RwLock<U256>,
    address: Address,
}

impl<M> LockedNonceManagerMiddleware<M>
where
    M: Middleware,
{
    /// Instantiates the nonce manager with a 0 nonce. The `address` should be the
    /// address which you'll be sending transactions from
    pub fn new(inner: M, address: Address) -> Self {
        Self {
            initialized: false.into(),
            nonce: RwLock::new(U256::zero()),
            inner,
            address,
        }
    }

    /// initialize the nonce
    pub async fn initialize_nonce(
        &self,
        block: Option<BlockId>,
    ) -> Result<U256, NonceManagerError<M>> {
        self.get_or_init_nonce(block).await
    }

    /// Returns the next nonce to be used
    pub async fn next(&self) -> U256 {
        let read_guard = self.nonce.read().await;
        *read_guard
    }

    async fn get_or_init_nonce(
        &self,
        block: Option<BlockId>,
    ) -> Result<U256, NonceManagerError<M>> {
        // initialize the nonce the first time the manager is called
        if !self.initialized.load(Ordering::SeqCst) {
            let nonce = self
                .inner
                .get_transaction_count(self.address, block)
                .await
                .map_err(FromErr::from)?;
            let mut write_guard = self.nonce.write().await;
            *write_guard = nonce;
            self.initialized.store(true, Ordering::SeqCst);
        }
        // return current nonce
        Ok(self.next().await)
    }
}

#[derive(Error, Debug)]
/// Thrown when an error happens at the Nonce Manager
pub enum NonceManagerError<M: Middleware> {
    /// Thrown when the internal middleware errors
    #[error("{0}")]
    MiddlewareError(M::Error),
}

impl<M: Middleware> FromErr<M::Error> for NonceManagerError<M> {
    fn from(src: M::Error) -> Self {
        NonceManagerError::MiddlewareError(src)
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<M> Middleware for LockedNonceManagerMiddleware<M>
where
    M: Middleware,
{
    type Error = NonceManagerError<M>;
    type Provider = M::Provider;
    type Inner = M;

    fn inner(&self) -> &M {
        &self.inner
    }

    async fn fill_transaction(
        &self,
        tx: &mut TypedTransaction,
        block: Option<BlockId>,
    ) -> Result<(), Self::Error> {
        let mut write_guard = self.nonce.write().await;
        let mut nonce = *write_guard;

        if tx.nonce().is_none() {
            nonce = self.get_or_init_nonce(block).await?;
            tx.set_nonce(nonce);
        }

        let res = self
            .inner()
            .fill_transaction(tx, block)
            .await
            .map_err(FromErr::from)?;

        *write_guard = nonce + U256::from(1u32);

        Ok(res)
    }

    /// Signs and broadcasts the transaction. The optional parameter `block` can be passed so that
    /// gas cost and nonce calculations take it into account. For simple transactions this can be
    /// left to `None`.
    async fn send_transaction<T: Into<TypedTransaction> + Send + Sync>(
        &self,
        tx: T,
        block: Option<BlockId>,
    ) -> Result<PendingTransaction<'_, Self::Provider>, Self::Error> {
        let mut tx = tx.into();

        let mut write_guard = self.nonce.write().await;
        let mut nonce = *write_guard;

        if tx.nonce().is_none() {
            nonce = self.get_or_init_nonce(block).await?;
            tx.set_nonce(nonce);
        }

        let res = match self.inner.send_transaction(tx.clone(), block).await {
            Ok(tx_hash) => Ok(tx_hash),
            Err(err) => {
                let current_nonce = self.get_transaction_count(self.address, block).await?;
                if current_nonce > nonce {
                    *write_guard = current_nonce;
                    tx.set_nonce(nonce);
                    self.inner
                        .send_transaction(tx, block)
                        .await
                        .map_err(FromErr::from)
                } else {
                    // propagate the error otherwise
                    Err(FromErr::from(err))
                }
            }
        }?;

        *write_guard = nonce + U256::from(1u32);

        Ok(res)
    }
}
