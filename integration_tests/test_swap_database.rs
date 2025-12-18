//! Test to reproduce InvalidTagEncoding(64) error when reading swaps from database
//! 
//! This test creates a swap, saves it to a database using the same method as the
//! actual code (heed::SerdeBincode), and then tries to read it back to reproduce
//! the InvalidTagEncoding(64) error.

use coinshift::state::State;
use coinshift::types::{Address, Swap, SwapDirection, SwapId, SwapState, SwapTxId, ParentChainType};
use bitcoin::Amount;
use std::error::Error;
use hex;
use tempfile::TempDir;
use sneed::{
    Env,
    env::Error as EnvError,
};
use heed::{EnvFlags, EnvOpenOptions};

/// Test that reproduces the InvalidTagEncoding(64) error when reading from database
fn test_swap_database_read_error() -> Result<(), Box<dyn Error>> {
    println!("=== Testing Swap Database Read Error ===");
    
    // Create a temporary directory for the database
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("data.mdb");
    
    // Create the database directory (data.mdb is actually a directory, not a file)
    std::fs::create_dir_all(&db_path)?;
    println!("Database path: {:?}", db_path);
    
    // Create swap ID from the error log
    let swap_id_bytes = hex::decode("0edcc53e3ea55d58ac526c2a87ad2ee1d4e26dedb3f8f973995b132bd12a9f41")?;
    let mut swap_id_array = [0u8; 32];
    swap_id_array.copy_from_slice(&swap_id_bytes);
    let swap_id = SwapId(swap_id_array);
    
    println!("Swap ID: {}", swap_id);
    
    // Create a swap similar to what's created in fill_swap_test
    let l2_recipient = Address([1u8; 20]);
    
    let swap = Swap::new(
        swap_id,
        SwapDirection::L2ToL1,
        ParentChainType::Signet,
        SwapTxId::Hash32([0u8; 32]), // Dummy TXID
        None, // Use default confirmations (3 for Signet)
        Some(l2_recipient),
        Amount::from_sat(5_000_000), // L2 amount: 5,000,000 sats
        Some("tb1qvu95383kuvcwxv2z0lextheej4sdwz4vvxvkxg".to_string()), // L1 recipient address
        Some(Amount::from_sat(5_000_000)), // L1 amount: 5,000,000 sats
        100, // created_at_height
        Some(200), // expires_at_height
    );
    
    println!("\nSwap details:");
    println!("  ID: {}", swap.id);
    println!("  Direction: {:?}", swap.direction);
    println!("  Parent Chain: {:?}", swap.parent_chain);
    println!("  State: {:?}", swap.state);
    println!("  L2 Amount: {} sats", swap.l2_amount.to_sat());
    println!("  L1 Amount: {:?} sats", swap.l1_amount.map(|a| a.to_sat()));
    
    // Open database environment (similar to how Node does it)
    println!("\n=== Opening Database ===");
    let env = {
        let mut env_open_opts = EnvOpenOptions::new();
        env_open_opts.max_dbs(State::NUM_DBS);
        env_open_opts.map_size(1024 * 1024 * 1024); // 1GB
        
        // Use fast flags similar to Node
        let fast_flags = EnvFlags::WRITE_MAP
            | EnvFlags::MAP_ASYNC
            | EnvFlags::NO_SYNC
            | EnvFlags::NO_META_SYNC
            | EnvFlags::NO_READ_AHEAD
            | EnvFlags::NO_TLS;
        unsafe { env_open_opts.flags(fast_flags) };
        
        unsafe { Env::open(&env_open_opts, &db_path) }
            .map_err(|e| {
                let env_err: EnvError = e.into();
                format!("Failed to open database: {}", env_err)
            })?
    };
    println!("✓ Database opened successfully");
    
    // Create State instance
    println!("\n=== Creating State ===");
    let state = State::new(&env)
        .map_err(|e| format!("Failed to create State: {}", e))?;
    println!("✓ State created successfully");
    
    // Save swap to database
    println!("\n=== Saving Swap to Database ===");
    {
        let mut rwtxn = env.write_txn()
            .map_err(|e| {
                let env_err: EnvError = e.into();
                format!("Failed to create write transaction: {}", env_err)
            })?;
        
        // Use the same method as State::save_swap
        state.swaps
            .put(&mut rwtxn, &swap.id, &swap)
            .map_err(|e| format!("Failed to save swap: {}", e))?;
        
        println!("✓ Swap saved to database");
        
        // Commit the transaction
        rwtxn.commit()
            .map_err(|e| format!("Failed to commit transaction: {}", e))?;
        println!("✓ Transaction committed");
    }
    
    // Try to read swap back from database
    println!("\n=== Reading Swap from Database ===");
    {
        let rtxn = env.read_txn()
            .map_err(|e| {
                let env_err: EnvError = e.into();
                format!("Failed to create read transaction: {}", env_err)
            })?;
        
        // This is where the error should occur
        match state.swaps.try_get(&rtxn, &swap.id) {
            Ok(Some(read_swap)) => {
                println!("✓ Swap read successfully from database!");
                println!("  Read swap ID: {}", read_swap.id);
                println!("  Read swap direction: {:?}", read_swap.direction);
                println!("  Read swap L2 amount: {} sats", read_swap.l2_amount.to_sat());
                
                // Verify the swap matches
                assert_eq!(swap.id, read_swap.id, "Swap ID should match");
                assert_eq!(swap.direction, read_swap.direction, "Direction should match");
                assert_eq!(swap.l2_amount, read_swap.l2_amount, "L2 amount should match");
                assert_eq!(swap.l1_amount, read_swap.l1_amount, "L1 amount should match");
                
                println!("✓ All fields match!");
            }
            Ok(None) => {
                return Err("Swap was not found in database (but it should be there)".into());
            }
            Err(e) => {
                let err_str = format!("{}", e);
                let err_debug = format!("{:?}", e);
                
                println!("✗ Failed to read swap from database");
                println!("  Error: {}", e);
                println!("  Error display: {}", err_str);
                println!("  Error debug: {}", err_debug);
                
                // Check if it's the InvalidTagEncoding error
                if err_str.contains("InvalidTagEncoding") || err_debug.contains("InvalidTagEncoding") {
                    println!("\n  ⚠ REPRODUCED InvalidTagEncoding ERROR!");
                    println!("  This matches the error from the database read failure in the actual code.");
                    println!("  The swap was saved but cannot be deserialized when reading back.");
                    
                    // This is the expected error, so we return it as a test failure
                    return Err(format!("InvalidTagEncoding error reproduced: {}", e).into());
                } else {
                    return Err(format!("Unexpected error reading swap: {}", e).into());
                }
            }
        }
    }
    
    println!("\n=== Test Complete ===");
    Ok(())
}

/// Test with multiple swaps to see if the issue is consistent
fn test_multiple_swaps() -> Result<(), Box<dyn Error>> {
    println!("\n=== Testing Multiple Swaps ===");
    
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("data.mdb");
    
    // Create the database directory
    std::fs::create_dir_all(&db_path)?;
    
    // Open database
    let env = {
        let mut env_open_opts = EnvOpenOptions::new();
        env_open_opts.max_dbs(State::NUM_DBS);
        env_open_opts.map_size(1024 * 1024 * 1024);
        
        let fast_flags = EnvFlags::WRITE_MAP
            | EnvFlags::MAP_ASYNC
            | EnvFlags::NO_SYNC
            | EnvFlags::NO_META_SYNC
            | EnvFlags::NO_READ_AHEAD
            | EnvFlags::NO_TLS;
        unsafe { env_open_opts.flags(fast_flags) };
        
        unsafe { Env::open(&env_open_opts, &db_path) }
            .map_err(|e| format!("Failed to open database: {}", EnvError::from(e)))?
    };
    
    let state = State::new(&env)
        .map_err(|e| format!("Failed to create State: {}", e))?;
    
    // Create multiple swaps with different configurations
    let test_cases = vec![
        ("Pending swap", SwapState::Pending),
        ("WaitingConfirmations swap", SwapState::WaitingConfirmations(1, 3)),
        ("ReadyToClaim swap", SwapState::ReadyToClaim),
        ("Completed swap", SwapState::Completed),
        ("Cancelled swap", SwapState::Cancelled),
    ];
    
    let mut saved_swaps = Vec::new();
    
    // Save all swaps
    {
        let mut rwtxn = env.write_txn()
            .map_err(|e| format!("Failed to create write transaction: {}", EnvError::from(e)))?;
        
        for (i, (name, state_variant)) in test_cases.iter().enumerate() {
            // Create unique swap ID for each swap
            let mut swap_id_bytes = [0u8; 32];
            swap_id_bytes[0] = i as u8;
            let swap_id = SwapId(swap_id_bytes);
            
            let mut swap = Swap::new(
                swap_id,
                SwapDirection::L2ToL1,
                ParentChainType::Signet,
                SwapTxId::Hash32([0u8; 32]),
                None,
                Some(Address([1u8; 20])),
                Amount::from_sat(1_000_000),
                Some("tb1qtest".to_string()),
                Some(Amount::from_sat(1_000_000)),
                100,
                Some(200),
            );
            swap.state = state_variant.clone();
            
            println!("Saving {}...", name);
            state.swaps
                .put(&mut rwtxn, &swap.id, &swap)
                .map_err(|e| format!("Failed to save {}: {}", name, e))?;
            
            saved_swaps.push((name, swap));
        }
        
        rwtxn.commit()
            .map_err(|e| format!("Failed to commit: {}", e))?;
        println!("✓ All swaps saved");
    }
    
    // Try to read all swaps back
    {
        let rtxn = env.read_txn()
            .map_err(|e| format!("Failed to create read transaction: {}", EnvError::from(e)))?;
        
        for (name, original_swap) in &saved_swaps {
            println!("Reading {}...", name);
            match state.swaps.try_get(&rtxn, &original_swap.id) {
                Ok(Some(read_swap)) => {
                    println!("  ✓ {} read successfully", name);
                    assert_eq!(original_swap.id, read_swap.id);
                    assert_eq!(original_swap.state, read_swap.state);
                }
                Ok(None) => {
                    println!("  ✗ {} not found", name);
                    return Err(format!("{} was not found in database", name).into());
                }
                Err(e) => {
                    println!("  ✗ {} failed to read: {}", name, e);
                    let err_str = format!("{}", e);
                    if err_str.contains("InvalidTagEncoding") {
                        println!("    ⚠ InvalidTagEncoding error for {}", name);
                        return Err(format!("InvalidTagEncoding error for {}: {}", name, e).into());
                    } else {
                        return Err(format!("Unexpected error for {}: {}", name, e).into());
                    }
                }
            }
        }
    }
    
    println!("✓ All swaps read successfully");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();
    
    println!("Running swap database read tests...\n");
    
    // Test 1: Single swap with the exact ID from the error log
    match test_swap_database_read_error() {
        Ok(()) => {
            println!("\n✓ Test 1 passed: Swap was saved and read successfully");
        }
        Err(e) => {
            let err_str = format!("{}", e);
            if err_str.contains("InvalidTagEncoding") {
                println!("\n✗ Test 1 failed: InvalidTagEncoding error reproduced");
                println!("  This confirms the serialization/deserialization issue.");
            } else {
                println!("\n✗ Test 1 failed: {}", e);
            }
            return Err(e);
        }
    }
    
    // Test 2: Multiple swaps with different states
    match test_multiple_swaps() {
        Ok(()) => {
            println!("\n✓ Test 2 passed: All swaps were saved and read successfully");
        }
        Err(e) => {
            let err_str = format!("{}", e);
            if err_str.contains("InvalidTagEncoding") {
                println!("\n✗ Test 2 failed: InvalidTagEncoding error reproduced");
            } else {
                println!("\n✗ Test 2 failed: {}", e);
            }
            return Err(e);
        }
    }
    
    println!("\n=== All Tests Complete ===");
    Ok(())
}
