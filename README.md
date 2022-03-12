# ethers-nonce

[ethers-rs](https://github.com/gakonst/ethers-rs) middleware for managing nonces locally.
This is useful when you need to send out multiple txs without needing them to get confirmed.
This is a fork of the [original implementation](https://github.com/gakonst/ethers-rs/blob/2b178e9cf79572a5905bb2d45a0061bf058b9675/ethers-middleware/src/nonce_manager.rs) that is shipped with the ethers-middleware
package.

The change in implementation here is:
1. Local nonce is only incremented when `fill_transaction` or `send_transaction` call succeeds.
2. Synchronization is done using [`tokio::sync::RwLock`](https://docs.rs/tokio/1.17.0/tokio/sync/struct.RwLock.html).

Why?
1. In original implementation the local nonce gets incremented regardless of `fill_transaction` or
   `send_transaction` failing which can lead to situations where the nonce will get incremented
   even though that nonce tx is not sent resulting in all txs after the failed nonce tx to be
   prevented from being mined.
