use bitcoin::{
    absolute::LockTime,
    hashes::Hash,
    secp256k1::{Keypair, Secp256k1},
    transaction::{TxIn, Version},
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxOut, Txid, WPubkeyHash, WScriptHash,
    Witness, XOnlyPublicKey,
};
use serde::{Deserialize, Serialize};
use std::vec;
use tonic_lnd::lnrpc::Utxo;

#[derive(Debug)]
pub enum FestivusError {
    NotEnoughBitcoin,
    ReqwestError,
}

impl std::fmt::Display for FestivusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            FestivusError::NotEnoughBitcoin => write!(f, "Not enough bitcoin in wallet."),
            FestivusError::ReqwestError => write!(f, "Could not receive fee rates."),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecommendedFess {
    fastest_fee: u64,
    half_hour_fee: u64,
    hour_fee: u64,
    economy_fee: u64,
    minimum_fee: u64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct ProjectedFees {
    fastest_fee: (u64, u64),
    half_hour_fee: (u64, u64),
    hour_fee: (u64, u64),
    economy_fee: (u64, u64),
    minimum_fee: (u64, u64),
}

pub fn calculate_fee(mut utxos: Vec<Utxo>, amount: u64) -> Result<ProjectedFees, FestivusError> {
    // Sort the UTXO's for largest first selection.
    // This is the default coin selection algorithm for LND
    utxos.sort_by(|a, b| b.amount_sat.cmp(&a.amount_sat));

    // The coins selected for the transaction.
    let mut coins = Vec::<Utxo>::new();
    // If the coins fulfill requirement for the transaction.
    let mut amount_remaining: i64 = amount as i64;

    // Iterate over the provided utxos and select the coins used for the transaction.
    utxos.iter().for_each(|utxo| {
        if amount_remaining > 0 {
            coins.push(utxo.clone());
            amount_remaining -= utxo.amount_sat;
        }
    });

    // Not enough BTC for the transaction in the wallet.
    if amount_remaining > 0 {
        return Err(FestivusError::NotEnoughBitcoin);
    }

    // Convert the UTXO to TxIn to get the transaction weight.
    let txin = coins
        .iter()
        .map(|utxo| utxo_to_txin(utxo.clone()))
        .collect::<Vec<TxIn>>();

    // Create a random taproot keypair for the ouput.
    let secp = Secp256k1::new();
    let mut rand = rand::thread_rng();
    let (secret_key, _) = secp.generate_keypair(&mut rand);
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let (pubkey, _) = XOnlyPublicKey::from_keypair(&keypair);

    // The channel open output, P2WSH
    let funding_output = TxOut {
        value: Amount::from_sat(amount),
        script_pubkey: ScriptBuf::new_p2wsh(&WScriptHash::hash(&[0u8; 68])),
    };

    // The change output, LND defaults to P2TR.
    let change_output = TxOut {
        value: Amount::from_sat(amount_remaining.abs() as u64),
        script_pubkey: ScriptBuf::new_p2tr(&secp, pubkey, None),
    };

    // The final valid transaction.
    let txn = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: txin,
        output: vec![funding_output, change_output],
    };

    // Get the weight
    let weight = txn.weight();
    let virtual_bytes = weight.to_vbytes_ceil();

    // Get fees
    let fees = reqwest::blocking::get("https://mempool.space/api/v1/fees/recommended")
        .map_err(|_| FestivusError::ReqwestError)?
        .json::<RecommendedFess>()
        .map_err(|_| FestivusError::ReqwestError)?;

    // Calc total amount
    Ok(ProjectedFees {
        fastest_fee: (virtual_bytes * fees.fastest_fee, fees.fastest_fee),
        half_hour_fee: (virtual_bytes * fees.half_hour_fee, fees.half_hour_fee),
        hour_fee: (virtual_bytes * fees.hour_fee, fees.hour_fee),
        economy_fee: (virtual_bytes * fees.economy_fee, fees.economy_fee),
        minimum_fee: (virtual_bytes * fees.minimum_fee, fees.minimum_fee),
    })
}

/// Convert the `tonic_lnd::Utxo` type to the `bitcoin::Utxo type`
fn utxo_to_txin(utxo: Utxo) -> TxIn {
    let previous_output = match utxo.outpoint {
        Some(op) => OutPoint {
            txid: Txid::hash(&op.txid_bytes),
            vout: op.output_index,
        },
        None => OutPoint::default(),
    };

    let script_sig = match utxo.address_type {
        // P2TR: Generate a random keypair for script_sig.
        4 => {
            let secp = Secp256k1::new();

            let mut rand = rand::thread_rng();
            let (secret_key, _) = secp.generate_keypair(&mut rand);
            let keypair = Keypair::from_secret_key(&secp, &secret_key);
            let (pubkey, _) = XOnlyPublicKey::from_keypair(&keypair);

            ScriptBuf::new_p2tr(&secp, pubkey, None)
        }
        // P2WPKH
        _ => ScriptBuf::new_p2wpkh(&WPubkeyHash::hash(utxo.pk_script.as_bytes())),
    };

    let witness = match utxo.address_type {
        // P2TR
        4 => {
            let mut witness = Witness::default();
            witness.push(&[0u8; 142]);
            witness
        }
        // P2WPKH
        _ => {
            let mut witness = Witness::default();
            witness.push(&[0u8; 128]);
            witness.push(&[0u8; 66]);
            witness
        }
    };

    TxIn {
        previous_output,
        script_sig,
        sequence: Sequence::ZERO,
        witness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_fee() {
        let mut utxo_one = Utxo::default();
        utxo_one.amount_sat = Amount::from_btc(3.6).unwrap().to_sat() as i64;
        utxo_one.outpoint = Some(tonic_lnd::lnrpc::OutPoint {
            txid_bytes: Txid::all_zeros().to_string().as_bytes().to_owned(),
            txid_str: Txid::all_zeros().to_string(),
            output_index: 1,
        });
        utxo_one.address_type = 4;

        let mut utxo_two = Utxo::default();
        utxo_two.amount_sat = Amount::from_btc(1.2).unwrap().to_sat() as i64;

        let utxos = vec![utxo_one, utxo_two];

        let fees = calculate_fee(utxos, 19_000);

        assert_eq!(fees.is_ok(), true)
    }

    #[test]
    fn not_enough_btc() {
        let mut utxo_one = Utxo::default();
        utxo_one.amount_sat = Amount::from_sat(10_000).to_sat() as i64;
        utxo_one.outpoint = Some(tonic_lnd::lnrpc::OutPoint {
            txid_bytes: Txid::all_zeros().to_string().as_bytes().to_owned(),
            txid_str: Txid::all_zeros().to_string(),
            output_index: 1,
        });
        utxo_one.address_type = 4;

        let mut utxo_two = Utxo::default();
        utxo_two.amount_sat = Amount::from_sat(5_000).to_sat() as i64;

        let utxos = vec![utxo_one, utxo_two];

        let fees = calculate_fee(utxos, 19_000);

        assert_eq!(fees.is_err(), true)
    }
}
