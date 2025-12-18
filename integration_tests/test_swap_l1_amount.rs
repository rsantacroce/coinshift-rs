//! Test to isolate the l1_amount Option<Amount> issue

use coinshift::types::{Address, Swap, SwapDirection, SwapId, SwapState, SwapTxId, ParentChainType};
use bitcoin::Amount;
use bincode;
use serde::{Deserialize, Serialize};
use std::error::Error;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct SwapUpToL1Amount {
    id: SwapId,
    direction: SwapDirection,
    parent_chain: ParentChainType,
    l1_txid: SwapTxId,
    required_confirmations: u32,
    state: SwapState,
    l2_recipient: Option<Address>,
    #[serde(with = "bitcoin::amount::serde::as_sat")]
    l2_amount: bitcoin::Amount,
    l1_recipient_address: Option<String>,
    #[serde(with = "bitcoin::amount::serde::as_sat::opt")]
    l1_amount: Option<bitcoin::Amount>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let swap_id = SwapId([0u8; 32]);
    
    let test_swap = SwapUpToL1Amount {
        id: swap_id,
        direction: SwapDirection::L2ToL1,
        parent_chain: ParentChainType::Signet,
        l1_txid: SwapTxId::Hash32([0u8; 32]),
        required_confirmations: 3,
        state: SwapState::Pending,
        l2_recipient: Some(Address([1u8; 20])),
        l2_amount: Amount::from_sat(100000),
        l1_recipient_address: Some("bc1qtest".to_string()),
        l1_amount: Some(Amount::from_sat(50000)),
    };
    
    println!("Testing Swap with l1_amount (Option<Amount> with custom serde)...");
    let serialized = bincode::serialize(&test_swap)?;
    println!("Serialized {} bytes", serialized.len());
    
    match bincode::deserialize::<SwapUpToL1Amount>(&serialized) {
        Ok(_) => {
            println!("✓ Swap with l1_amount works!");
        }
        Err(e) => {
            println!("✗ Swap with l1_amount failed: {}", e);
            println!("Error: {:?}", e);
            return Err(e.into());
        }
    }
    
    // Now test the full Swap struct
    println!("\nTesting full Swap struct...");
    let full_swap = Swap::new(
        swap_id,
        SwapDirection::L2ToL1,
        ParentChainType::Signet,
        SwapTxId::Hash32([0u8; 32]),
        None,
        Some(Address([1u8; 20])),
        Amount::from_sat(100000),
        Some("bc1qtest".to_string()),
        Some(Amount::from_sat(50000)),
        100,
        Some(200),
    );
    
    let full_serialized = bincode::serialize(&full_swap)?;
    match bincode::deserialize::<Swap>(&full_serialized) {
        Ok(_) => println!("✓ Full Swap works!"),
        Err(e) => {
            println!("✗ Full Swap failed: {}", e);
            return Err(e.into());
        }
    }
    
    Ok(())
}
