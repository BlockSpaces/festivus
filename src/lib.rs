use bitcoin::{
    absolute::LockTime,
    hashes::Hash,
    secp256k1::{Keypair, Secp256k1},
    transaction::{self, InputWeightPrediction},
    Amount, ScriptBuf, Transaction, TxOut, WScriptHash,
    XOnlyPublicKey, Txid
};
use serde::{Deserialize, Serialize};
use std::vec;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FestivusError {
    #[error("Not enough Bitcoin in the wallet.")]
    NotEnoughBitcoin,
    #[error("Error getting recommended fees.")]
    ReqwestError,
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

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct ProjectedFees {
    pub fastest_fee: (u64, u64),
    pub half_hour_fee: (u64, u64),
    pub hour_fee: (u64, u64),
    pub economy_fee: (u64, u64),
    pub minimum_fee: (u64, u64),
}

#[derive(Debug, Default, Clone)]
pub enum FestivusAddressType {
    #[default]
    Taproot,
    Other
}

#[derive(Debug, Clone, Default)]
pub struct FestivusUtxo {
    pub address_type: FestivusAddressType,
    pub address: String,
    pub amount_sat: i64,
    pub pk_script: String,
    pub outpoint: Option<FestivusOutpoint>
}

#[derive(Debug, Clone)]
pub struct FestivusOutpoint {
    pub txid_bytes: Vec<u8>,
    pub txid: String,
    pub output_index: i32
}

impl Default for FestivusOutpoint {
    fn default() -> Self {
        Self {
            txid_bytes: Txid::all_zeros().to_byte_array().to_vec(),
            txid: Txid::all_zeros().to_string(),
            output_index: 1
        }
    }
}

pub async fn calculate_fee(utxos: Option<Vec<FestivusUtxo>>, amount: i64) -> Result<ProjectedFees, FestivusError> {
    // Create a random taproot keypair for the ouput.
    let secp = Secp256k1::new();
    let mut rand = rand::thread_rng();
    let (secret_key, _) = secp.generate_keypair(&mut rand);
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let (pubkey, _) = XOnlyPublicKey::from_keypair(&keypair);

    // The channel open output, P2WSH
    let funding_output = TxOut {
        value: Amount::from_sat(336),
        script_pubkey: ScriptBuf::new_p2wsh(&WScriptHash::hash(&[0u8; 43])),
    };

    // The change output, LND defaults to P2TR.
    let change_output = TxOut {
        value: Amount::from_sat(256),
        script_pubkey: ScriptBuf::new_p2tr(&secp, pubkey, None),
    };

    // The final valid transaction.
    let txn = Transaction {
        version: transaction::Version::TWO,
        lock_time: LockTime::ZERO,
        input: Vec::with_capacity(2),
        output: vec![funding_output, change_output],
    };

    let utxos = match utxos {
        Some(u) => u,
        None => {
            let mut utxo = FestivusUtxo::default();
            utxo.amount_sat = amount;
            utxo.outpoint = Some(FestivusOutpoint::default());
            utxo.address_type = FestivusAddressType::Taproot;
            vec![utxo]
        }
    };
    let inputs = predict_weight_for_inputs(utxos, amount)?;

    let weight = transaction::predict_weight(inputs, txn.script_pubkey_lens());

    let virtual_bytes = weight.to_vbytes_ceil();

    // Get fees
    let fees = reqwest::get("https://mempool.space/api/v1/fees/recommended")
        .await
        .map_err(|_| FestivusError::ReqwestError)?
        .json::<RecommendedFess>()
        .await
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

fn predict_weight_for_inputs(mut utxos: Vec<FestivusUtxo>, amount: i64) -> Result<Vec<InputWeightPrediction>, FestivusError> {
    // Sort the UTXO's for largest first selection.
    // This is the default coin selection algorithm for LND
    utxos.sort_by(|a, b| b.amount_sat.cmp(&a.amount_sat));

    // The coins selected for the transaction.
    let mut coins = Vec::<FestivusUtxo>::new();
    // If the coins fulfill requirement for the transaction.
    let mut amount_remaining = amount;

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

    // From each UTXO used, get the weight prediction.
    let txin = coins
        .iter()
        .map(|utxo| {
            match utxo.address_type {
                FestivusAddressType::Taproot => InputWeightPrediction::P2TR_KEY_DEFAULT_SIGHASH,
                FestivusAddressType::Other => InputWeightPrediction::P2WPKH_MAX,
            }
        })
        .collect::<Vec<InputWeightPrediction>>();

    
    Ok(txin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn calc_fee_p2tr_inputs() {
        let mut utxo_one = FestivusUtxo::default();
        utxo_one.amount_sat = Amount::from_btc(3.6).unwrap().to_sat() as i64;
        utxo_one.outpoint = Some(FestivusOutpoint::default());
        utxo_one.address_type = FestivusAddressType::Taproot;

        let mut utxo_two = FestivusUtxo::default();
        utxo_two.amount_sat = Amount::from_btc(1.2).unwrap().to_sat() as i64;

        let utxos = vec![utxo_one, utxo_two];

        let fees = calculate_fee(Some(utxos), 19_000).await;

        assert_eq!(fees.is_ok(), true)
    }

    #[tokio::test]
    async fn calc_fee_p2wkh_inputs() {
        let mut utxo_one = FestivusUtxo::default();
        utxo_one.amount_sat = Amount::from_btc(3.6).unwrap().to_sat() as i64;
        utxo_one.outpoint = Some(FestivusOutpoint::default());
        utxo_one.address_type = FestivusAddressType::Other;

        let mut utxo_two = FestivusUtxo::default();
        utxo_two.amount_sat = Amount::from_btc(1.2).unwrap().to_sat()as i64;

        let utxos = vec![utxo_one, utxo_two];

        let fees = calculate_fee(Some(utxos), 19_000).await;

        assert_eq!(fees.is_ok(), true)
    }

    #[tokio::test]
    async fn no_utxos() {
        let fees = calculate_fee(None, 19_000).await;

        assert_eq!(fees.is_ok(), true)
    }

    #[tokio::test]
    async fn calc_fee_two_inputs() {
        let mut utxo_one = FestivusUtxo::default();
        utxo_one.amount_sat = Amount::from_btc(1.0).unwrap().to_sat() as i64;
        utxo_one.outpoint = Some(FestivusOutpoint::default());
        utxo_one.address_type = FestivusAddressType::Other;

        let mut utxo_two = FestivusUtxo::default();
        utxo_two.amount_sat = Amount::from_btc(0.5).unwrap().to_sat() as i64;
        utxo_two.address_type = FestivusAddressType::Other;

        let utxos = vec![utxo_one, utxo_two];

        let fees = calculate_fee(Some(utxos), 125_000_000).await;

        assert_eq!(fees.is_ok(), true)
    }

    #[tokio::test]
    async fn not_enough_btc() {
        let mut utxo_one = FestivusUtxo::default();
        utxo_one.amount_sat = Amount::from_sat(10_000).to_sat() as i64;
        utxo_one.outpoint = Some(FestivusOutpoint::default());
        utxo_one.address_type = FestivusAddressType::Taproot;

        let mut utxo_two = FestivusUtxo::default();
        utxo_two.amount_sat = Amount::from_sat(5_000).to_sat() as i64;

        let utxos = vec![utxo_one, utxo_two];

        let fees = calculate_fee(Some(utxos), 19_000).await;

        assert_eq!(fees.is_err(), true)
    }
}
