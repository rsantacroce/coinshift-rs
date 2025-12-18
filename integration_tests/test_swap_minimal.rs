//! Minimal test to isolate the swap serialization issue

use coinshift::types::{Address, Swap, SwapDirection, SwapId, SwapState, SwapTxId, ParentChainType};
use bitcoin::Amount;
use bincode;
use serde::{Deserialize, Serialize};
use std::error::Error;

// Test struct without custom serde attributes
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
struct TestSwap {
    id: SwapId,
    direction: SwapDirection,
    parent_chain: ParentChainType,
    l1_txid: SwapTxId,
    required_confirmations: u32,
    state: SwapState,
    l2_recipient: Option<Address>,
    l2_amount_sat: u64,  // Plain u64 instead of Amount with custom serde
}

fn main() -> Result<(), Box<dyn Error>> {
    let swap_id = SwapId([0u8; 32]);
    
    let test_swap = TestSwap {
        id: swap_id,
        direction: SwapDirection::L2ToL1,
        parent_chain: ParentChainType::Signet,
        l1_txid: SwapTxId::Hash32([0u8; 32]),
        required_confirmations: 3,
        state: SwapState::Pending,
        l2_recipient: Some(Address([1u8; 20])),
        l2_amount_sat: 100000,
    };
    
    println!("Testing minimal swap struct...");
    let serialized = bincode::serialize(&test_swap)?;
    println!("Serialized {} bytes", serialized.len());
    
    let deserialized: TestSwap = bincode::deserialize(&serialized)?;
    assert_eq!(test_swap, deserialized);
    println!("✓ Minimal swap struct works!");
    
    // Now test the full Swap struct
    println!("\nTesting full Swap struct...");
    let swap = Swap::new(
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
    
    let serialized = bincode::serialize(&swap)?;
    println!("Full Swap serialized {} bytes", serialized.len());
    
    match bincode::deserialize::<Swap>(&serialized) {
        Ok(deserialized) => {
            println!("✓ Full Swap struct works!");
            assert_eq!(swap.id, deserialized.id);
            assert_eq!(swap.state, deserialized.state);
        }
        Err(e) => {
            println!("✗ Full Swap struct failed: {}", e);
            return Err(e.into());
        }
    }
    
    Ok(())
}
