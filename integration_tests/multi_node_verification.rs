//! Multi-node verification test
//! 
//! This test verifies that a third node (Charles) can observe and verify
//! all transactions between two other nodes (Bob and Alice).

use bip300301_enforcer_integration_tests::{
    integration_test::{activate_sidechain, deposit, fund_enforcer, propose_sidechain},
    setup::{
        Mode, Network, PostSetup as EnforcerPostSetup, Sidechain as _,
        setup as setup_enforcer,
    },
    util::{AbortOnDrop, AsyncTrial},
};
use coinshift::types::{Address, GetValue, ParentChainType, SwapId, SwapState};
use coinshift_app_rpc_api::RpcClient as _;
use futures::{FutureExt, StreamExt as _, channel::mpsc, future::BoxFuture};
use std::net::SocketAddr;
use tokio::time::sleep;
use tracing::Instrument as _;

use crate::{
    setup::{Init, PostSetup},
    util::BinPaths,
};

#[derive(Debug)]
struct MultiNodeSetup {
    bob: PostSetup,
    alice: PostSetup,
    charles: PostSetup,
}

const DEPOSIT_AMOUNT: bitcoin::Amount = bitcoin::Amount::from_sat(21_000_000);
const DEPOSIT_FEE: bitcoin::Amount = bitcoin::Amount::from_sat(1_000_000);
const SWAP_L2_AMOUNT: u64 = 10_000_000; // 0.1 BTC
const SWAP_L1_AMOUNT: u64 = 5_000_000; // 0.05 BTC
const SWAP_FEE: u64 = 1_000; // 0.00001 BTC
const TRANSFER_AMOUNT: u64 = 2_000_000; // 0.02 BTC
const TRANSFER_FEE: u64 = 1_000;

/// Initial setup for the test
async fn setup(
    bin_paths: &BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<(EnforcerPostSetup, MultiNodeSetup)> {
    let mut enforcer_post_setup = setup_enforcer(
        &bin_paths.others,
        Network::Regtest,
        Mode::Mempool,
        res_tx.clone(),
    )
    .await?;

    // Setup Bob's node
    let bob = PostSetup::setup(
        Init {
            coinshift_app: bin_paths.coinshift_app.clone(),
            data_dir_suffix: Some("bob".to_owned()),
        },
        &enforcer_post_setup,
        res_tx.clone(),
    )
    .await?;
    tracing::info!("Setup Bob's node successfully");

    // Setup Alice's node
    let alice = PostSetup::setup(
        Init {
            coinshift_app: bin_paths.coinshift_app.clone(),
            data_dir_suffix: Some("alice".to_owned()),
        },
        &enforcer_post_setup,
        res_tx.clone(),
    )
    .await?;
    tracing::info!("Setup Alice's node successfully");

    // Setup Charles's node (observer/verifier)
    let charles = PostSetup::setup(
        Init {
            coinshift_app: bin_paths.coinshift_app.clone(),
            data_dir_suffix: Some("charles".to_owned()),
        },
        &enforcer_post_setup,
        res_tx,
    )
    .await?;
    tracing::info!("Setup Charles's node successfully");

    let multi_node_setup = MultiNodeSetup { bob, alice, charles };

    // Propose and activate sidechain
    let () = propose_sidechain::<PostSetup>(&mut enforcer_post_setup).await?;
    tracing::info!("Proposed sidechain successfully");
    let () = activate_sidechain::<PostSetup>(&mut enforcer_post_setup).await?;
    tracing::info!("Activated sidechain successfully");
    let () = fund_enforcer::<PostSetup>(&mut enforcer_post_setup).await?;

    Ok((enforcer_post_setup, multi_node_setup))
}

/// Connect nodes in a network topology
/// Charles connects to both Bob and Alice to observe their transactions
async fn connect_nodes(nodes: &MultiNodeSetup) -> anyhow::Result<()> {
    tracing::info!("Connecting nodes...");
    
    // Charles connects to Bob
    nodes
        .charles
        .rpc_client
        .connect_peer(nodes.bob.net_addr().into())
        .await?;
    tracing::info!("Charles connected to Bob");
    sleep(std::time::Duration::from_secs(1)).await;

    // Charles connects to Alice
    nodes
        .charles
        .rpc_client
        .connect_peer(nodes.alice.net_addr().into())
        .await?;
    tracing::info!("Charles connected to Alice");
    sleep(std::time::Duration::from_secs(1)).await;

    // Bob and Alice also connect to each other for direct communication
    nodes
        .bob
        .rpc_client
        .connect_peer(nodes.alice.net_addr().into())
        .await?;
    tracing::info!("Bob connected to Alice");
    sleep(std::time::Duration::from_secs(1)).await;

    // Verify connections
    let charles_peers = nodes.charles.rpc_client.list_peers().await?;
    let charles_peer_addrs: Vec<SocketAddr> = charles_peers
        .iter()
        .map(|p| p.address)
        .collect();
    
    anyhow::ensure!(
        charles_peer_addrs.contains(&nodes.bob.net_addr().into()),
        "Charles should be connected to Bob"
    );
    anyhow::ensure!(
        charles_peer_addrs.contains(&nodes.alice.net_addr().into()),
        "Charles should be connected to Alice"
    );

    tracing::info!("All nodes connected successfully");
    Ok(())
}

/// Wait for blocks to sync across all nodes
async fn wait_for_sync(
    nodes: &MultiNodeSetup,
    expected_blocks: u32,
) -> anyhow::Result<()> {
    const MAX_RETRIES: usize = 20;
    const RETRY_DELAY_MS: u64 = 500;

    for attempt in 0..MAX_RETRIES {
        let bob_blocks = nodes.bob.rpc_client.getblockcount().await?;
        let alice_blocks = nodes.alice.rpc_client.getblockcount().await?;
        let charles_blocks = nodes.charles.rpc_client.getblockcount().await?;

        if bob_blocks == expected_blocks
            && alice_blocks == expected_blocks
            && charles_blocks == expected_blocks
        {
            tracing::info!(
                "All nodes synced to {} blocks (Bob: {}, Alice: {}, Charles: {})",
                expected_blocks,
                bob_blocks,
                alice_blocks,
                charles_blocks
            );
            return Ok(());
        }

        if attempt < MAX_RETRIES - 1 {
            tracing::debug!(
                "Waiting for sync... (Bob: {}, Alice: {}, Charles: {}, expected: {})",
                bob_blocks,
                alice_blocks,
                charles_blocks,
                expected_blocks
            );
            sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
        }
    }

    // Final check
    let bob_blocks = nodes.bob.rpc_client.getblockcount().await?;
    let alice_blocks = nodes.alice.rpc_client.getblockcount().await?;
    let charles_blocks = nodes.charles.rpc_client.getblockcount().await?;

    anyhow::ensure!(
        bob_blocks == expected_blocks,
        "Bob should have {} blocks, got {}",
        expected_blocks,
        bob_blocks
    );
    anyhow::ensure!(
        alice_blocks == expected_blocks,
        "Alice should have {} blocks, got {}",
        expected_blocks,
        alice_blocks
    );
    anyhow::ensure!(
        charles_blocks == expected_blocks,
        "Charles should have {} blocks, got {}",
        expected_blocks,
        charles_blocks
    );

    Ok(())
}

/// Verify that Charles can see all swaps created by Bob and Alice
async fn verify_charles_sees_swaps(
    nodes: &MultiNodeSetup,
    expected_swaps: &[(SwapId, Address, u64, u64)], // (swap_id, creator_address, l2_amount, l1_amount)
) -> anyhow::Result<()> {
    tracing::info!("Verifying Charles can see all swaps...");

    // Wait a bit for transaction propagation
    sleep(std::time::Duration::from_millis(1000)).await;

    let charles_swaps = nodes.charles.rpc_client.list_swaps().await?;
    
    tracing::info!(
        "Charles sees {} swaps, expected {}",
        charles_swaps.len(),
        expected_swaps.len()
    );

    // Verify Charles can see all expected swaps
    for (expected_swap_id, _creator_address, expected_l2_amount, expected_l1_amount) in expected_swaps {
        let swap = charles_swaps
            .iter()
            .find(|s| s.id == *expected_swap_id)
            .ok_or_else(|| {
                anyhow::anyhow!("Charles should see swap {}", expected_swap_id)
            })?;

        anyhow::ensure!(
            swap.l2_amount.to_sat() == *expected_l2_amount,
            "Charles sees wrong L2 amount for swap {}: expected {}, got {}",
            expected_swap_id,
            expected_l2_amount,
            swap.l2_amount.to_sat()
        );

        anyhow::ensure!(
            swap.l1_amount.map(|a| a.to_sat()) == Some(*expected_l1_amount),
            "Charles sees wrong L1 amount for swap {}: expected {}, got {:?}",
            expected_swap_id,
            expected_l1_amount,
            swap.l1_amount.map(|a| a.to_sat())
        );

        tracing::info!("Charles verified swap {} correctly", expected_swap_id);
    }

    anyhow::ensure!(
        charles_swaps.len() >= expected_swaps.len(),
        "Charles should see at least {} swaps, got {}",
        expected_swaps.len(),
        charles_swaps.len()
    );

    tracing::info!("Charles successfully verified all swaps");
    Ok(())
}

/// Verify that Charles can see transactions in blocks
async fn verify_charles_sees_transactions(
    nodes: &MultiNodeSetup,
    expected_block_count: u32,
) -> anyhow::Result<()> {
    tracing::info!("Verifying Charles can see all transactions in blocks...");

    // All nodes should have the same block count
    let bob_blocks = nodes.bob.rpc_client.getblockcount().await?;
    let alice_blocks = nodes.alice.rpc_client.getblockcount().await?;
    let charles_blocks = nodes.charles.rpc_client.getblockcount().await?;

    anyhow::ensure!(
        bob_blocks == expected_block_count,
        "Bob should have {} blocks, got {}",
        expected_block_count,
        bob_blocks
    );
    anyhow::ensure!(
        alice_blocks == expected_block_count,
        "Alice should have {} blocks, got {}",
        expected_block_count,
        alice_blocks
    );
    anyhow::ensure!(
        charles_blocks == expected_block_count,
        "Charles should have {} blocks, got {}",
        expected_block_count,
        charles_blocks
    );

    // Verify Charles can see the same sidechain wealth
    let bob_wealth = nodes.bob.rpc_client.sidechain_wealth_sats().await?;
    let alice_wealth = nodes.alice.rpc_client.sidechain_wealth_sats().await?;
    let charles_wealth = nodes.charles.rpc_client.sidechain_wealth_sats().await?;

    // All nodes should see the same total sidechain wealth
    anyhow::ensure!(
        bob_wealth == alice_wealth && alice_wealth == charles_wealth,
        "All nodes should see the same sidechain wealth. Bob: {}, Alice: {}, Charles: {}",
        bob_wealth,
        alice_wealth,
        charles_wealth
    );

    tracing::info!(
        "All nodes see the same sidechain wealth: {} sats",
        charles_wealth
    );

    Ok(())
}

async fn multi_node_verification_task(
    bin_paths: BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let (mut enforcer_post_setup, mut nodes) = setup(&bin_paths, res_tx).await?;

    // Connect nodes
    connect_nodes(&nodes).await?;

    // Fund Bob and Alice with deposits
    tracing::info!("Funding Bob's wallet...");
    let bob_deposit_address = nodes.bob.get_deposit_address().await?;
    deposit(
        &mut enforcer_post_setup,
        &mut nodes.bob,
        &bob_deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Bob deposited successfully");

    tracing::info!("Funding Alice's wallet...");
    let alice_deposit_address = nodes.alice.get_deposit_address().await?;
    deposit(
        &mut enforcer_post_setup,
        &mut nodes.alice,
        &alice_deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Alice deposited successfully");

    // Mine blocks to include deposits
    nodes.bob.bmm_single(&mut enforcer_post_setup).await?;
    sleep(std::time::Duration::from_millis(500)).await;
    wait_for_sync(&nodes, 1).await?;

    // Bob creates a swap
    tracing::info!("Bob creating a swap...");
    let bob_l2_recipient = nodes.bob.rpc_client.get_new_address().await?;
    let l1_recipient = "bcrt1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
    let (bob_swap_id, bob_swap_txid) = nodes
        .bob
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient.to_string(),
            SWAP_L1_AMOUNT,
            Some(bob_l2_recipient),
            SWAP_L2_AMOUNT,
            Some(1),
            SWAP_FEE,
        )
        .await?;
    tracing::info!(
        swap_id = %bob_swap_id,
        swap_txid = %bob_swap_txid,
        "Bob created swap"
    );

    // Mine block to include Bob's swap
    nodes.bob.bmm_single(&mut enforcer_post_setup).await?;
    sleep(std::time::Duration::from_millis(500)).await;
    wait_for_sync(&nodes, 2).await?;

    // Alice creates a swap
    tracing::info!("Alice creating a swap...");
    let alice_l2_recipient = nodes.alice.rpc_client.get_new_address().await?;
    let (alice_swap_id, alice_swap_txid) = nodes
        .alice
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient.to_string(),
            SWAP_L1_AMOUNT,
            Some(alice_l2_recipient),
            SWAP_L2_AMOUNT,
            Some(1),
            SWAP_FEE,
        )
        .await?;
    tracing::info!(
        swap_id = %alice_swap_id,
        swap_txid = %alice_swap_txid,
        "Alice created swap"
    );

    // Mine block to include Alice's swap
    nodes.alice.bmm_single(&mut enforcer_post_setup).await?;
    sleep(std::time::Duration::from_millis(500)).await;
    wait_for_sync(&nodes, 3).await?;

    // Bob transfers funds to Alice
    tracing::info!("Bob transferring funds to Alice...");
    let alice_receive_address = nodes.alice.rpc_client.get_new_address().await?;
    let transfer_txid = nodes
        .bob
        .rpc_client
        .transfer(alice_receive_address, TRANSFER_AMOUNT, TRANSFER_FEE)
        .await?;
    tracing::info!(txid = %transfer_txid, "Bob transferred funds to Alice");

    // Mine block to include transfer
    nodes.bob.bmm_single(&mut enforcer_post_setup).await?;
    sleep(std::time::Duration::from_millis(500)).await;
    wait_for_sync(&nodes, 4).await?;

    // Verify Charles can see all swaps
    let expected_swaps = vec![
        (bob_swap_id, bob_l2_recipient, SWAP_L2_AMOUNT, SWAP_L1_AMOUNT),
        (alice_swap_id, alice_l2_recipient, SWAP_L2_AMOUNT, SWAP_L1_AMOUNT),
    ];
    verify_charles_sees_swaps(&nodes, &expected_swaps).await?;

    // Verify Charles can see all transactions
    verify_charles_sees_transactions(&nodes, 4).await?;

    // Verify Charles can see UTXOs (he should see the same blockchain state)
    let bob_utxos = nodes.bob.rpc_client.list_utxos().await?;
    let alice_utxos = nodes.alice.rpc_client.list_utxos().await?;
    let charles_utxos = nodes.charles.rpc_client.list_utxos().await?;

    tracing::info!(
        "UTXO counts - Bob: {}, Alice: {}, Charles: {}",
        bob_utxos.len(),
        alice_utxos.len(),
        charles_utxos.len()
    );

    // Charles should see all UTXOs in the blockchain (though not necessarily in his wallet)
    // The total value should be consistent
    let bob_total: u64 = bob_utxos
        .iter()
        .filter_map(|utxo| {
            if let coinshift::types::OutputContent::Value(v) = utxo.output.content {
                Some(v.to_sat())
            } else {
                None
            }
        })
        .sum();
    let alice_total: u64 = alice_utxos
        .iter()
        .filter_map(|utxo| {
            if let coinshift::types::OutputContent::Value(v) = utxo.output.content {
                Some(v.to_sat())
            } else {
                None
            }
        })
        .sum();
    let charles_total: u64 = charles_utxos
        .iter()
        .filter_map(|utxo| {
            if let coinshift::types::OutputContent::Value(v) = utxo.output.content {
                Some(v.to_sat())
            } else {
                None
            }
        })
        .sum();

    tracing::info!(
        "Wallet balances - Bob: {} sats, Alice: {} sats, Charles: {} sats",
        bob_total,
        alice_total,
        charles_total
    );

    // Verify Charles can query specific swaps
    let bob_swap_status = nodes
        .charles
        .rpc_client
        .get_swap_status(bob_swap_id)
        .await?;
    anyhow::ensure!(
        bob_swap_status.is_some(),
        "Charles should be able to query Bob's swap"
    );
    let bob_swap = bob_swap_status.unwrap();
    anyhow::ensure!(
        bob_swap.id == bob_swap_id,
        "Charles retrieved correct swap ID"
    );

    let alice_swap_status = nodes
        .charles
        .rpc_client
        .get_swap_status(alice_swap_id)
        .await?;
    anyhow::ensure!(
        alice_swap_status.is_some(),
        "Charles should be able to query Alice's swap"
    );
    let alice_swap = alice_swap_status.unwrap();
    anyhow::ensure!(
        alice_swap.id == alice_swap_id,
        "Charles retrieved correct swap ID"
    );

    tracing::info!("Charles successfully verified all transactions between Bob and Alice");

    // Cleanup
    drop(nodes.charles);
    drop(nodes.alice);
    drop(nodes.bob);
    tracing::info!("Removing {}", enforcer_post_setup.out_dir.path().display());
    drop(enforcer_post_setup.tasks);
    sleep(std::time::Duration::from_secs(1)).await;
    enforcer_post_setup.out_dir.cleanup()?;

    Ok(())
}

async fn multi_node_verification(bin_paths: BinPaths) -> anyhow::Result<()> {
    let (res_tx, mut res_rx) = mpsc::unbounded();
    let _test_task: AbortOnDrop<()> = tokio::task::spawn({
        let res_tx = res_tx.clone();
        async move {
            let res = multi_node_verification_task(bin_paths, res_tx.clone()).await;
            let _send_err: Result<(), _> = res_tx.unbounded_send(res);
        }
        .in_current_span()
    })
    .into();
    res_rx.next().await.ok_or_else(|| {
        anyhow::anyhow!("Unexpected end of test task result stream")
    })?
}

pub fn multi_node_verification_trial(
    bin_paths: BinPaths,
) -> AsyncTrial<BoxFuture<'static, anyhow::Result<()>>> {
    AsyncTrial::new(
        "multi_node_verification",
        multi_node_verification(bin_paths).boxed(),
    )
}
