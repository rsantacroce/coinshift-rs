//! L1 Transaction Report from BMM participants

use bitcoin::{BlockHash, hashes::Hash as _};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::authorization::{Signature, VerifyingKey};
use super::{Address, SwapId, SwapTxId};


/// L1 transaction report from a BMM participant
/// Reports that a swap transaction has been detected on the L1 chain
/// Note: Uses serde for serialization (stored in Body, which uses SerdeBincode in database)
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    Serialize,
)]
pub struct L1TransactionReport {
    pub swap_id: SwapId,
    pub l1_txid: SwapTxId,
    pub confirmations: u32,
    pub block_height: u32,
    #[serde(serialize_with = "serialize_block_hash", deserialize_with = "deserialize_block_hash")]
    pub mainchain_block_hash: BlockHash,
    /// BMM participant's verifying key (used to derive address)
    #[serde(serialize_with = "serialize_verifying_key", deserialize_with = "deserialize_verifying_key")]
    pub verifying_key: VerifyingKey,
    /// Signature of the report by the BMM participant
    #[serde(serialize_with = "serialize_signature", deserialize_with = "deserialize_signature")]
    pub signature: Signature,
}

// Custom serde serialization for VerifyingKey
fn serialize_verifying_key<S>(vk: &VerifyingKey, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serde::Serialize::serialize(&vk.to_bytes(), serializer)
}

fn deserialize_verifying_key<'de, D>(deserializer: D) -> Result<VerifyingKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let bytes: [u8; 32] = Deserialize::deserialize(deserializer)?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|e| serde::de::Error::custom(format!("Invalid verifying key: {}", e)))
}

// Custom serde serialization for Signature
fn serialize_signature<S>(sig: &Signature, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    // Serialize as Vec<u8> since [u8; 64] doesn't implement Serialize
    serde::Serialize::serialize(&sig.to_bytes().to_vec(), serializer)
}

fn deserialize_signature<'de, D>(deserializer: D) -> Result<Signature, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let bytes_vec: Vec<u8> = Deserialize::deserialize(deserializer)?;
    if bytes_vec.len() != 64 {
        return Err(serde::de::Error::custom(format!("Invalid signature length: expected 64 bytes, got {}", bytes_vec.len())));
    }
    let mut bytes = [0u8; 64];
    bytes.copy_from_slice(&bytes_vec);
    // Signature::from_bytes returns Signature directly (no Result)
    Ok(Signature::from_bytes(&bytes))
}

// Custom serde serialization for BlockHash
fn serialize_block_hash<S>(hash: &BlockHash, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serde::Serialize::serialize(&hash.as_byte_array(), serializer)
}

fn deserialize_block_hash<'de, D>(deserializer: D) -> Result<BlockHash, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let bytes: [u8; 32] = Deserialize::deserialize(deserializer)?;
    Ok(BlockHash::from_byte_array(bytes))
}

impl L1TransactionReport {
    /// Create a new L1 transaction report
    pub fn new(
        swap_id: SwapId,
        l1_txid: SwapTxId,
        confirmations: u32,
        block_height: u32,
        mainchain_block_hash: BlockHash,
        verifying_key: VerifyingKey,
        signature: Signature,
    ) -> Self {
        Self {
            swap_id,
            l1_txid,
            confirmations,
            block_height,
            mainchain_block_hash,
            verifying_key,
            signature,
        }
    }

    /// Get the address of the BMM participant who signed this report
    pub fn get_signer_address(&self) -> Address {
        use crate::authorization::get_address;
        get_address(&self.verifying_key)
    }

    /// Verify the signature of this report
    pub fn verify_signature(&self) -> Result<(), L1ReportError> {
        use borsh::BorshSerialize;
        use ed25519_dalek::Verifier;
        
        // Serialize the report data (without signature) for verification
        // Use borsh for deterministic serialization
        let report_data = ReportData {
            swap_id: self.swap_id,
            l1_txid: self.l1_txid.clone(),
            confirmations: self.confirmations,
            block_height: self.block_height,
            mainchain_block_hash: self.mainchain_block_hash,
        };
        
        let message = borsh::to_vec(&report_data)
            .map_err(|e| L1ReportError::SerializationError(format!("Failed to serialize report: {}", e)))?;
        
        // Verify signature using ed25519_dalek directly
        self.verifying_key
            .verify(&message, &self.signature)
            .map_err(|e| L1ReportError::SignatureVerificationFailed(format!("{}", e)))?;
        
        Ok(())
    }
}

/// Report data (without signature) for signing/verification
#[derive(borsh::BorshSerialize, Clone, Debug)]
struct ReportData {
    swap_id: SwapId,
    l1_txid: SwapTxId,
    confirmations: u32,
    block_height: u32,
    #[borsh(serialize_with = "serialize_block_hash_borsh")]
    mainchain_block_hash: BlockHash,
}

// Helper for Borsh serialization of BlockHash in ReportData
fn serialize_block_hash_borsh<W: borsh::io::Write>(
    hash: &BlockHash,
    writer: &mut W,
) -> borsh::io::Result<()> {
    borsh::BorshSerialize::serialize(&hash.as_byte_array(), writer)
}

/// Errors for L1 transaction reports
#[derive(Debug, thiserror::Error)]
pub enum L1ReportError {
    #[error("Signature verification failed: {0}")]
    SignatureVerificationFailed(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Invalid report: {0}")]
    InvalidReport(String),
}

