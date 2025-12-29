use borsh::{BorshDeserialize, BorshSerialize};

use bitcoin::{
    block::Header as BitcoinHeader,
    consensus::encode::deserialize,
    hashes::{Hash as _, sha256d},
};
use num_bigint::BigUint;
use num_traits::Zero as _;
use scrypt::{Params as ScryptParams, scrypt as scrypt_fn};

use crate::{
    state::Error,
    types::Swap,
};

/// Phase 1 LTC SPV proof.
///
/// Security note:
/// - This verifies internal consistency (tx output match + merkle inclusion in the provided header).
/// - It does NOT yet verify scrypt PoW / chainwork / best-chain selection.
///   That must be added before treating this as fully trustless finality.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct LtcSpvProofV1 {
    /// Raw transaction bytes (Bitcoin-style serialization; valid for LTC txs).
    pub tx: Vec<u8>,
    /// Merkle path sibling hashes from leaf to root.
    pub merkle_path: Vec<[u8; 32]>,
    /// Leaf index within the block's tx list.
    pub tx_index: u32,
    /// Raw 80-byte block header.
    pub header: [u8; 80],
}

/// Phase 1 LTC SPV proof (v2): includes a header chain so we can verify PoW and confirmations.
///
/// The `headers` vector must be a contiguous chain (by `prev_blockhash`), in ascending order.
/// The transaction is proven to be included in `headers[tx_header_index]`.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct LtcSpvProofV2 {
    /// Raw transaction bytes (Bitcoin-style serialization; valid for LTC txs).
    pub tx: Vec<u8>,
    /// Merkle path sibling hashes from leaf to root.
    pub merkle_path: Vec<[u8; 32]>,
    /// Leaf index within the block's tx list.
    pub tx_index: u32,
    /// Height of the first header in `headers` (must match an embedded checkpoint).
    pub start_height: u32,
    /// Contiguous LTC block headers (80 bytes each), oldest → newest.
    pub headers: Vec<[u8; 80]>,
    /// Index of the header in `headers` that contains the transaction.
    pub tx_header_index: u32,
    /// Minimum confirmations required w.r.t. the newest header in `headers`.
    pub min_confirmations: u32,
}

pub fn verify_ltc_spv_proof_v2(
    swap: &Swap,
    proof: &LtcSpvProofV2,
) -> Result<(), Error> {
    // Parse tx (Bitcoin consensus encoding is compatible for our purposes here)
    let tx: bitcoin::Transaction = deserialize(&proof.tx).map_err(|e| {
        Error::InvalidTransaction(format!("Invalid LTC tx encoding: {e}"))
    })?;
    let txid = tx.compute_txid();
    let txid_bytes = txid.to_byte_array();

    // If the swap already has a registered txid, it must match the proof txid.
    if let crate::types::SwapTxId::Hash32(expected) = swap.l1_txid {
        if expected != [0u8; 32] && txid_bytes != expected {
            return Err(Error::InvalidTransaction(
                "SPV proof txid does not match registered swap txid".to_string(),
            ));
        }
    }

    // Verify tx matches swap terms (recipient + amount).
    let expected_spk = parse_script_pubkey_descriptor(
        swap.l1_recipient_address.as_deref().ok_or_else(|| {
            Error::InvalidTransaction(
                "Swap missing l1_recipient_address".to_string(),
            )
        })?,
    )?;
    let expected_amount = swap.l1_amount.ok_or_else(|| {
        Error::InvalidTransaction("Swap missing l1_amount".to_string())
    })?;
    let expected_amount_sat = expected_amount.to_sat();

    let mut found = false;
    for out in &tx.output {
        if out.value.to_sat() == expected_amount_sat
            && out.script_pubkey.as_bytes() == expected_spk.as_slice()
        {
            found = true;
            break;
        }
    }
    if !found {
        return Err(Error::InvalidTransaction(
            "SPV proof tx does not match swap recipient/amount".to_string(),
        ));
    }

    // Validate header chain + PoW
    let tx_header_index: usize = proof.tx_header_index.try_into().map_err(|_| {
        Error::InvalidTransaction("tx_header_index out of range".to_string())
    })?;
    if proof.headers.is_empty() || tx_header_index >= proof.headers.len() {
        return Err(Error::InvalidTransaction(
            "Invalid headers / tx_header_index".to_string(),
        ));
    }
    if proof.min_confirmations == 0 {
        return Err(Error::InvalidTransaction(
            "min_confirmations must be > 0".to_string(),
        ));
    }
    // Confirmations = tip_index - tx_index + 1
    let tip_index = proof.headers.len() - 1;
    let confirmations = (tip_index - tx_header_index) as u32 + 1;
    if confirmations < proof.min_confirmations {
        return Err(Error::InvalidTransaction(format!(
            "Not enough LTC confirmations in proof: have {confirmations}, need {}",
            proof.min_confirmations
        )));
    }

    let headers: Vec<BitcoinHeader> = proof
        .headers
        .iter()
        .map(|h| {
            deserialize::<BitcoinHeader>(h).map_err(|e| {
                Error::InvalidTransaction(format!("Invalid LTC header encoding: {e}"))
            })
        })
        .collect::<Result<_, _>>()?;

    // Anchor: require the first header to match a known checkpoint at `start_height`.
    let first_hash = headers
        .first()
        .expect("checked non-empty")
        .block_hash()
        .to_byte_array();
    let expected_checkpoint = ltc_checkpoint_bytes(proof.start_height).ok_or_else(|| {
        Error::InvalidTransaction(format!(
            "Unsupported LTC start_height {} (no embedded checkpoint)",
            proof.start_height
        ))
    })?;
    if first_hash != expected_checkpoint {
        return Err(Error::InvalidTransaction(
            "First header does not match embedded LTC checkpoint".to_string(),
        ));
    }

    let pow_limit = BigUint::parse_bytes(
        b"00000fffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        16,
    )
    .expect("valid powLimit hex");

    for (i, header) in headers.iter().enumerate() {
        // Verify chain linkage (by block hash, not PoW hash)
        if i > 0 {
            let prev = &headers[i - 1];
            if header.prev_blockhash != prev.block_hash() {
                return Err(Error::InvalidTransaction(
                    "Header chain does not link".to_string(),
                ));
            }
        }

        // Verify difficulty/target (Bitcoin-style retarget, Litecoin params) + PoW.
        let height = proof.start_height.saturating_add(i as u32);
        verify_ltc_header_target_and_pow(
            &proof.headers[i],
            header,
            height,
            proof.start_height,
            &headers,
            &pow_limit,
        )?;
    }

    let tx_header = &headers[tx_header_index];

    // Verify merkle inclusion
    let computed_root =
        compute_merkle_root(&txid_bytes, proof.tx_index, &proof.merkle_path)?;
    if tx_header.merkle_root.to_byte_array() != computed_root {
        return Err(Error::InvalidTransaction(
            "Merkle proof does not match header merkle_root".to_string(),
        ));
    }

    Ok(())
}

fn parse_script_pubkey_descriptor(s: &str) -> Result<Vec<u8>, Error> {
    // Phase 1: require scriptPubKey hex to avoid chain-specific address decoding.
    // Accepted forms:
    // - "spk:<hex>"
    // - "<hex>"
    let hex_str = s.strip_prefix("spk:").unwrap_or(s);
    let bytes = hex::decode(hex_str).map_err(|e| {
        Error::InvalidTransaction(format!("Invalid scriptPubKey hex in swap: {e}"))
    })?;
    if bytes.is_empty() {
        return Err(Error::InvalidTransaction(
            "Empty scriptPubKey in swap".to_string(),
        ));
    }
    Ok(bytes)
}

fn compute_merkle_root(
    txid_bytes: &[u8; 32],
    mut index: u32,
    merkle_path: &[[u8; 32]],
) -> Result<[u8; 32], Error> {
    let mut h = sha256d::Hash::from_byte_array(*txid_bytes);
    for sib in merkle_path {
        let sib = sha256d::Hash::from_byte_array(*sib);
        let mut buf = [0u8; 64];
        if index % 2 == 0 {
            buf[..32].copy_from_slice(h.as_ref());
            buf[32..].copy_from_slice(sib.as_ref());
        } else {
            buf[..32].copy_from_slice(sib.as_ref());
            buf[32..].copy_from_slice(h.as_ref());
        }
        h = sha256d::Hash::hash(&buf);
        index /= 2;
    }
    Ok(h.to_byte_array())
}

fn verify_ltc_header_target_and_pow(
    header_bytes: &[u8; 80],
    header: &BitcoinHeader,
    height: u32,
    start_height: u32,
    headers: &[BitcoinHeader],
    pow_limit: &BigUint,
) -> Result<(), Error> {
    // Litecoin mainnet uses Bitcoin-style retarget with:
    // - nPowTargetTimespan = 302400 (3.5 days)
    // - nPowTargetSpacing  = 150 (2.5 minutes)
    // - interval = 2016
    const TARGET_TIMESPAN: u32 = 302_400;
    const TARGET_SPACING: u32 = 150;
    const INTERVAL: u32 = TARGET_TIMESPAN / TARGET_SPACING; // 2016

    // Enforce expected nBits for this height (relative to the provided anchored chain).
    // We treat the checkpoint header itself as trusted, and only enforce rules for subsequent headers.
    if height > start_height {
        let idx = (height - start_height) as usize;
        let expected_bits = if height % INTERVAL != 0 {
            // No retarget: bits must match previous block
            headers[idx - 1].bits.to_consensus()
        } else {
            // Retarget: compute based on the last interval
            if idx < INTERVAL as usize {
                return Err(Error::InvalidTransaction(
                    "Proof does not include enough headers to validate retarget".to_string(),
                ));
            }
            let first = &headers[idx - INTERVAL as usize];
            let last = &headers[idx - 1];
            let actual_timespan = last
                .time
                .saturating_sub(first.time)
                .clamp(TARGET_TIMESPAN / 4, TARGET_TIMESPAN * 4);
            let last_target = compact_to_biguint(last.bits.to_consensus())?;
            let mut new_target =
                (last_target * BigUint::from(actual_timespan))
                    / BigUint::from(TARGET_TIMESPAN);
            if &new_target > pow_limit {
                new_target = pow_limit.clone();
            }
            biguint_to_compact(&new_target)?
        };

        let got_bits = header.bits.to_consensus();
        if got_bits != expected_bits {
            return Err(Error::InvalidTransaction(
                "Header nBits does not match expected retarget result".to_string(),
            ));
        }
    }

    // Verify scrypt PoW <= target(bits)
    let target = compact_to_biguint(header.bits.to_consensus())?;
    if &target > pow_limit {
        return Err(Error::InvalidTransaction(
            "Header target exceeds powLimit".to_string(),
        ));
    }

    // Litecoin PoW is scrypt(N=1024, r=1, p=1, dkLen=32) over the 80-byte header.
    let params = ScryptParams::new(10, 1, 1, 32).map_err(|e| {
        Error::InvalidTransaction(format!("Invalid scrypt params: {e}"))
    })?;
    let mut out = [0u8; 32];
    scrypt_fn(header_bytes, header_bytes, &params, &mut out).map_err(|e| {
        Error::InvalidTransaction(format!("scrypt failed: {e}"))
    })?;

    // Convert PoW hash to big-endian bytes for comparison.
    let mut pow_be = out;
    pow_be.reverse();
    let pow_int = BigUint::from_bytes_be(&pow_be);
    if pow_int > target {
        return Err(Error::InvalidTransaction(
            "Header PoW does not meet target".to_string(),
        ));
    }
    Ok(())
}

fn compact_to_biguint(bits: u32) -> Result<BigUint, Error> {
    let exponent = (bits >> 24) as u32;
    let mantissa = bits & 0x007f_ffff;
    let negative = (bits & 0x0080_0000) != 0;
    if negative {
        return Err(Error::InvalidTransaction("Negative compact target".to_string()));
    }
    if mantissa == 0 {
        return Err(Error::InvalidTransaction("Zero compact target".to_string()));
    }
    // target = mantissa * 256^(exponent-3)
    let mut target = BigUint::from(mantissa);
    if exponent <= 3 {
        let shift = 8 * (3 - exponent);
        target >>= shift;
    } else {
        let shift = 8 * (exponent - 3);
        target <<= shift;
    }
    Ok(target)
}

fn biguint_to_compact(target: &BigUint) -> Result<u32, Error> {
    if target.is_zero() {
        return Err(Error::InvalidTransaction("Zero target".to_string()));
    }
    let bytes = target.to_bytes_be();
    // Determine exponent and mantissa (3 bytes).
    let mut exponent = bytes.len() as u32;
    let mantissa: u32;
    if bytes.len() <= 3 {
        mantissa = {
            let mut v = 0u32;
            for b in bytes.iter() {
                v = (v << 8) | (*b as u32);
            }
            v << (8 * (3 - bytes.len()))
        };
        exponent = 3;
    } else {
        // mantissa = first 3 bytes
        mantissa = ((bytes[0] as u32) << 16) | ((bytes[1] as u32) << 8) | (bytes[2] as u32);
    }
    // If mantissa's highest bit is set, shift down and increase exponent.
    let mut mantissa = mantissa;
    if mantissa & 0x0080_0000 != 0 {
        mantissa >>= 8;
        exponent += 1;
    }
    Ok((exponent << 24) | (mantissa & 0x007f_ffff))
}

fn ltc_checkpoint_bytes(height: u32) -> Option<[u8; 32]> {
    // Internal (consensus) byte order (little-endian) for `bitcoin::BlockHash::to_byte_array()`.
    match height {
        // From Litecoin Core `src/chainparams.cpp` (master) and blockchair API for newer anchors.
        721_000 => Some(hex_bytes_32(
            "e540989a758adc4116743ee6a235b2ea14b7759dd93b46e27894dfe14d7b8a19",
        )),
        2_520_000 => Some(hex_bytes_32(
            "c10d5c2f1294e5f74bea4a084b89a5179d4e7ac6e045c8b95b7c978af40ecd54",
        )),
        3_000_000 => Some(hex_bytes_32(
            "fd45dd6f1df580504322734666ff37e46831a5159b302e0422b9d531bb2c23ed",
        )),
        _ => None,
    }
}

fn hex_bytes_32(s: &str) -> [u8; 32] {
    let v = hex::decode(s).expect("valid hex checkpoint");
    v.try_into().expect("checkpoint must be 32 bytes")
}


