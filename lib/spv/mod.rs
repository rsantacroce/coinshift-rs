//! Minimal SPV proof parsing/verification for Phase 1 swap proofs.
//!
//! Important: Phase 1 is intended for LTC + ZEC (transparent). This module defines a
//! versioned proof container so we can evolve formats without breaking consensus.

use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    state::Error,
    types::{ParentChainType, Swap, SwapTxId},
};

mod ltc;
pub mod ltc_builder;

/// Versioned SPV proof container.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum SpvProof {
    LtcV1(ltc::LtcSpvProofV1),
    LtcV2(ltc::LtcSpvProofV2),
    // Reserved for Phase 1+; ZEC verification is more complex (Equihash).
    ZecV1(ZecSpvProofV1),
}

/// Placeholder format for ZEC until we implement proper verification.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct ZecSpvProofV1 {
    pub _reserved: Vec<u8>,
}

pub fn verify_swap_spv_proof(
    swap: &Swap,
    proof_bytes: &[u8],
) -> Result<(), Error> {
    let proof = SpvProof::try_from_slice(proof_bytes).map_err(|e| {
        Error::InvalidTransaction(format!("Invalid SPV proof encoding: {e}"))
    })?;

    match (swap.parent_chain, proof) {
        (ParentChainType::LTC, SpvProof::LtcV1(_p)) => Err(Error::InvalidTransaction(
            "LTC SPV proof v1 is disabled (requires header-chain PoW verification). Use LtcV2."
                .to_string(),
        )),
        (ParentChainType::LTC, SpvProof::LtcV2(p)) => ltc::verify_ltc_spv_proof_v2(swap, &p),
        (ParentChainType::ZEC, SpvProof::ZecV1(_p)) => Err(Error::InvalidTransaction(
            "ZEC SPV proof verification not implemented yet".to_string(),
        )),
        (chain, _) => Err(Error::InvalidTransaction(format!(
            "SPV proof type does not match swap parent chain: {:?}",
            chain
        ))),
    }
}

/// Extract the L1 txid from a proof blob, for indexing/auditing.
/// This is deterministic and safe to run in consensus code.
pub fn extract_l1_txid_from_proof(
    parent_chain: ParentChainType,
    proof_bytes: &[u8],
) -> Result<Option<SwapTxId>, Error> {
    let proof = SpvProof::try_from_slice(proof_bytes).map_err(|e| {
        Error::InvalidTransaction(format!("Invalid SPV proof encoding: {e}"))
    })?;
    match (parent_chain, proof) {
        (ParentChainType::LTC, SpvProof::LtcV2(p)) => {
            let tx: bitcoin::Transaction = bitcoin::consensus::encode::deserialize(&p.tx)
                .map_err(|e| Error::InvalidTransaction(format!("Invalid proof tx: {e}")))?;
            Ok(Some(SwapTxId::from_bitcoin_txid(&tx.compute_txid())))
        }
        _ => Ok(None),
    }
}


