# Festivus

Calculate a projected fee for a channel open with LND UTXOs

```
use tonic_lnd::walletrpc::ListUnspent;
use tonic_lnd::lnrpc::Utxo;

let projected_fees = calculate_fees(utxos, amount);

let (total_fees, sat_vbyte) = projected_fees.fastest_fee;
```
