# Festivus

Calculate a projected fee for a channel open with LND UTXOs

```
use tonic_lnd::walletrpc::ListUnspent;
use tonic_lnd::lnrpc::Utxo;

let projected_fees = calculate_fees(utxos, amount);

let (total_fees, sat_vbyte) = projected_fees.fastest_fee;
```

Festivus now supports both asynchronous and synchronous execution modes.

By default, the calculate_fee function operates asynchronously. This is suitable for most applications that require non-blocking operations.

If your application requires synchronous execution, you can enable the synchronous feature of festivus. To do this, add the following to your Cargo.toml:

[dependencies]
festivus = { git = "https://github.com/BlockSpaces/festivus", features = ["blocking"] }
This enables the calculate_fee function to operate in a blocking manner.

Testing:

The festivus crate includes tests for both asynchronous and synchronous modes. You can run tests for the default asynchronous mode using:
cargo test

To run tests for the synchronous mode, use:
cargo test --features "blocking"