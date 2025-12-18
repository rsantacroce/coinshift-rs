//! Test swap serialization field by field to find the exact issue

use coinshift::types::{Address, Swap, SwapDirection, SwapId, SwapState, SwapTxId, ParentChainType};
use bitcoin::Amount;
use bincode;
use serde::{Deserialize, Serialize};
use std::error::Error;

// Test structs that gradually add fields to find where it breaks
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct SwapFields1 {
    id: SwapId,
    direction: SwapDirection,
    parent_chain: ParentChainType,
    l1_txid: SwapTxId,
    required_confirmations: u32,
    state: SwapState,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct SwapFields2 {
    id: SwapId,
    direction: SwapDirection,
    parent_chain: ParentChainType,
    l1_txid: SwapTxId,
    required_confirmations: u32,
    state: SwapState,
    l2_recipient: Option<Address>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct SwapFields3 {
    id: SwapId,
    direction: SwapDirection,
    parent_chain: ParentChainType,
    l1_txid: SwapTxId,
    required_confirmations: u32,
    state: SwapState,
    l2_recipient: Option<Address>,
    #[serde(with = "bitcoin::amount::serde::as_sat")]
    l2_amount: bitcoin::Amount,
}

fn test_fields(fields: &str, data: &[u8]) -> Result<(), Box<dyn Error>> {
    println!("Testing {}...", fields);
    match fields {
        "1" => {
            let _deserialized: SwapFields1 = bincode::deserialize(data)?;
            println!("  ✓ Fields 1-6 work");
        }
        "2" => {
            let _deserialized: SwapFields2 = bincode::deserialize(data)?;
            println!("  ✓ Fields 1-7 work");
        }
        "3" => {
            let _deserialized: SwapFields3 = bincode::deserialize(data)?;
            println!("  ✓ Fields 1-8 work");
        }
        _ => {}
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let swap_id = SwapId([0u8; 32]);
    
    // Create a full swap
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
    
    let full_serialized = bincode::serialize(&swap)?;
    println!("Full Swap serialized {} bytes\n", full_serialized.len());
    
    // Test fields 1-6 (before l2_recipient)
    let fields1 = SwapFields1 {
        id: swap.id,
        direction: swap.direction,
        parent_chain: swap.parent_chain,
        l1_txid: swap.l1_txid.clone(),
        required_confirmations: swap.required_confirmations,
        state: swap.state.clone(),
    };
    let serialized1 = bincode::serialize(&fields1)?;
    test_fields("1", &serialized1)?;
    
    // Test fields 1-7 (before l2_amount with custom serde)
    let fields2 = SwapFields2 {
        id: swap.id,
        direction: swap.direction,
        parent_chain: swap.parent_chain,
        l1_txid: swap.l1_txid.clone(),
        required_confirmations: swap.required_confirmations,
        state: swap.state.clone(),
        l2_recipient: swap.l2_recipient,
    };
    let serialized2 = bincode::serialize(&fields2)?;
    test_fields("2", &serialized2)?;
    
    // Test fields 1-8 (with l2_amount custom serde)
    let fields3 = SwapFields3 {
        id: swap.id,
        direction: swap.direction,
        parent_chain: swap.parent_chain,
        l1_txid: swap.l1_txid.clone(),
        required_confirmations: swap.required_confirmations,
        state: swap.state.clone(),
        l2_recipient: swap.l2_recipient,
        l2_amount: swap.l2_amount,
    };
    let serialized3 = bincode::serialize(&fields3)?;
    test_fields("3", &serialized3)?;
    
    // Now try to deserialize the full swap
    println!("\nTesting full Swap struct...");
    match bincode::deserialize::<Swap>(&full_serialized) {
        Ok(_) => println!("  ✓ Full Swap works!"),
        Err(e) => {
            println!("  ✗ Full Swap failed: {}", e);
            println!("  Error: {:?}", e);
            
            // Try to see where it fails by checking byte positions
            println!("\n  Analyzing byte positions...");
            println!("  Fields 1-6 end at byte: {}", serialized1.len());
            println!("  Fields 1-7 end at byte: {}", serialized2.len());
            println!("  Fields 1-8 end at byte: {}", serialized3.len());
            println!("  Full swap length: {}", full_serialized.len());
            
            // Check if the issue is with Option<u64> (what Option<Amount> serializes to)
            println!("\n  Testing if issue is with Option<u64>...");
            let test_opt_u64 = Some(50000u64);
            let serialized_opt_u64 = bincode::serialize(&test_opt_u64)?;
            println!("  Option<u64> serialized to {} bytes", serialized_opt_u64.len());
            match bincode::deserialize::<Option<u64>>(&serialized_opt_u64) {
                Ok(_) => println!("  ✓ Option<u64> works standalone"),
                Err(e) => println!("  ✗ Option<u64> failed: {}", e),
            }
        }
    }
    
    Ok(())
}
