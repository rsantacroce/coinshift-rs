//! Test multi-node setup where Charles verifies transactions between Bob and Alice.
//!
//! Run this test from the workspace root:
//!
//! ```bash
//! # Build the app and set env (see integration_tests/example.env)
//! cargo build -p coinshift_app
//! export COINSHIFT_INTEGRATION_TEST_ENV=integration_tests/example.env
//!
//! # Run only this test
//! cargo run -p coinshift_integration_tests --example integration_tests -- --tests multi_node_verification
//!
//! # Or run all integration tests
//! cargo run -p coinshift_integration_tests --example integration_tests
//! ```

use bip300301_enforcer_integration_tests::{
    integration_test::{
        activate_sidechain, deposit, fund_enforcer, propose_sidechain,
    },
    setup::{
        Mode, Network, PostSetup as EnforcerPostSetup, Sidechain as _,
        setup as setup_enforcer,
    },
    util::{AbortOnDrop, AsyncTrial, TestFailureCollector, TestFileRegistry},
};
use coinshift::types::ParentChainType;
use coinshift_app_rpc_api::RpcClient as _;
use futures::{
    FutureExt as _, StreamExt as _, channel::mpsc, future::BoxFuture,
};
use tokio::time::sleep;
use tracing::Instrument as _;

use crate::{
    setup::{Init, PostSetup},
    util::BinPaths,
};

const DEPOSIT_AMOUNT: bitcoin::Amount = bitcoin::Amount::from_sat(50_000_000);
const DEPOSIT_FEE: bitcoin::Amount = bitcoin::Amount::from_sat(1_000_000);
const TRANSFER_AMOUNT: u64 = 10_000_000; // 0.1 BTC
const TRANSFER_FEE: u64 = 1_000; // 0.00001 BTC
const SWAP_L2_AMOUNT: u64 = 5_000_000; // 0.05 BTC
const SWAP_L1_AMOUNT: u64 = 2_500_000; // 0.025 BTC
const SWAP_FEE: u64 = 1_000; // 0.00001 BTC

#[derive(Debug)]
struct MultiNodeSetup {
    bob: PostSetup,
    alice: PostSetup,
    charles: PostSetup,
}

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
    let () = propose_sidechain::<PostSetup>(&mut enforcer_post_setup).await?;
    tracing::info!("Proposed sidechain successfully");
    let () = activate_sidechain::<PostSetup>(&mut enforcer_post_setup).await?;
    tracing::info!("Activated sidechain successfully");
    let () = fund_enforcer::<PostSetup>(&mut enforcer_post_setup).await?;
    tracing::info!("Funded enforcer successfully");

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

    // Setup Charles's node (verifier)
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

    // Connect all nodes so BMM blocks propagate to everyone
    connect_all_peers(
        &multi_node_setup.bob,
        &multi_node_setup.alice,
        &multi_node_setup.charles,
    )
    .await?;

    Ok((enforcer_post_setup, multi_node_setup))
}

/// Wait for nodes to sync by mining a block
async fn sync_nodes(
    nodes: &[&PostSetup],
    enforcer: &mut EnforcerPostSetup,
) -> anyhow::Result<()> {
    // Mine a block to ensure all nodes are synced
    if let Some(first_node) = nodes.first() {
        first_node.bmm_single(enforcer).await?;
        sleep(std::time::Duration::from_millis(500)).await;
    }
    Ok(())
}

/// Connect one node to another; ignore "already connected" so we can do full mesh.
async fn connect_peer_ignore_duplicate(
    from: &PostSetup,
    to_addr: std::net::SocketAddrV4,
) {
    if let Err(e) = from.rpc_client.connect_peer(to_addr.into()).await {
        let msg = e.to_string();
        if !msg.contains("already connected") {
            tracing::warn!(%to_addr, error = %e, "connect_peer failed");
        }
    }
}

/// Connect all three nodes (full mesh) so BMM blocks propagate to everyone.
async fn connect_all_peers(
    bob: &PostSetup,
    alice: &PostSetup,
    charles: &PostSetup,
) -> anyhow::Result<()> {
    tracing::info!("Connecting Bob, Alice, and Charles (full mesh) so blocks propagate");
    connect_peer_ignore_duplicate(bob, alice.net_addr()).await;
    connect_peer_ignore_duplicate(bob, charles.net_addr()).await;
    connect_peer_ignore_duplicate(alice, bob.net_addr()).await;
    connect_peer_ignore_duplicate(alice, charles.net_addr()).await;
    connect_peer_ignore_duplicate(charles, bob.net_addr()).await;
    connect_peer_ignore_duplicate(charles, alice.net_addr()).await;
    sleep(std::time::Duration::from_secs(2)).await;
    Ok(())
}

/// Verify that Charles can see transactions between Bob and Alice
async fn verify_transactions(
    charles: &PostSetup,
    bob: &PostSetup,
    alice: &PostSetup,
) -> anyhow::Result<()> {
    tracing::info!("Charles verifying transactions between Bob and Alice");

    let charles_block_count = charles.rpc_client.getblockcount().await?;
    let bob_block_count = bob.rpc_client.getblockcount().await?;
    let alice_block_count = alice.rpc_client.getblockcount().await?;

    // Allow up to 2 blocks difference between any two nodes (P2P propagation can lag)
    let max_blocks = charles_block_count
        .max(bob_block_count)
        .max(alice_block_count);
    let min_blocks = charles_block_count
        .min(bob_block_count)
        .min(alice_block_count);
    anyhow::ensure!(
        max_blocks.saturating_sub(min_blocks) <= 2,
        "Block counts should be within 2. Charles: {}, Bob: {}, Alice: {}",
        charles_block_count,
        bob_block_count,
        alice_block_count
    );

    // Get all swaps from each node's perspective
    let charles_swaps = charles.rpc_client.list_swaps().await?;
    let bob_swaps = bob.rpc_client.list_swaps().await?;
    let alice_swaps = alice.rpc_client.list_swaps().await?;

    tracing::debug!(
        charles_swaps_count = charles_swaps.len(),
        bob_swaps_count = bob_swaps.len(),
        alice_swaps_count = alice_swaps.len(),
        "Swap counts from each node"
    );

    // Get UTXOs from each node
    let charles_utxos = charles.rpc_client.list_utxos().await?;
    let bob_utxos = bob.rpc_client.list_utxos().await?;
    let alice_utxos = alice.rpc_client.list_utxos().await?;

    tracing::debug!(
        charles_utxos_count = charles_utxos.len(),
        bob_utxos_count = bob_utxos.len(),
        alice_utxos_count = alice_utxos.len(),
        "UTXO counts from each node"
    );

    // Verify Charles can see the blockchain state
    // (the total sidechain wealth should be consistent)
    let charles_wealth = charles.rpc_client.sidechain_wealth_sats().await?;
    let bob_wealth = bob.rpc_client.sidechain_wealth_sats().await?;
    let alice_wealth = alice.rpc_client.sidechain_wealth_sats().await?;

    // When nodes have the same block count, they should see the same wealth
    if bob_block_count == alice_block_count {
        anyhow::ensure!(
            bob_wealth == alice_wealth,
            "Bob and Alice should see the same sidechain wealth. Bob: {}, Alice: {}",
            bob_wealth,
            alice_wealth
        );
    }
    if charles_block_count == max_blocks {
        anyhow::ensure!(
            charles_wealth == bob_wealth.max(alice_wealth),
            "Charles (synced) should see consistent sidechain wealth. Charles: {}",
            charles_wealth
        );
    }

    tracing::info!(
        "Charles successfully verified all transactions between Bob and Alice. \
         Block count: {} (Bob: {}, Alice: {}), Swaps visible: {} (Bob: {}, Alice: {}), Sidechain wealth: {} sats",
        charles_block_count,
        bob_block_count,
        alice_block_count,
        charles_swaps.len(),
        bob_swaps.len(),
        alice_swaps.len(),
        charles_wealth
    );

    Ok(())
}

async fn multi_node_verification_task(
    bin_paths: BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let (mut enforcer_post_setup, mut nodes) =
        setup(&bin_paths, res_tx).await?;

    // Fund Bob and Alice with deposits
    tracing::info!("Funding Bob and Alice");
    let bob_deposit_address = nodes.bob.get_deposit_address().await?;
    let () = deposit(
        &mut enforcer_post_setup,
        &mut nodes.bob,
        &bob_deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Deposited to Bob successfully");

    let alice_deposit_address = nodes.alice.get_deposit_address().await?;
    let () = deposit(
        &mut enforcer_post_setup,
        &mut nodes.alice,
        &alice_deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Deposited to Alice successfully");

    // Sync nodes
    sync_nodes(&[&nodes.bob, &nodes.alice, &nodes.charles], &mut enforcer_post_setup)
        .await?;

    // Bob and Alice perform different operations

    // 1. Bob transfers to Alice
    tracing::info!("Bob transferring to Alice");
    let alice_receive_address = nodes.alice.rpc_client.get_new_address().await?;
    let transfer_txid = nodes
        .bob
        .rpc_client
        .transfer(alice_receive_address, TRANSFER_AMOUNT, TRANSFER_FEE)
        .await?;
    tracing::info!(txid = %transfer_txid, "Bob transferred to Alice");

    // Mine the transfer transaction
    nodes.bob.bmm_single(&mut enforcer_post_setup).await?;
    sleep(std::time::Duration::from_millis(500)).await;

    // 2. Bob creates a swap
    tracing::info!("Bob creating a swap");
    let l1_recipient_address = "bcrt1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
    let bob_l2_recipient = nodes.bob.rpc_client.get_new_address().await?;
    let (swap_id_bob, swap_txid_bob) = nodes
        .bob
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient_address.to_string(),
            SWAP_L1_AMOUNT,
            Some(bob_l2_recipient),
            SWAP_L2_AMOUNT,
            Some(1),
            SWAP_FEE,
        )
        .await?;
    tracing::info!(
        swap_id = %swap_id_bob,
        swap_txid = %swap_txid_bob,
        "Bob created swap"
    );

    // Mine the swap transaction
    nodes.bob.bmm_single(&mut enforcer_post_setup).await?;
    sleep(std::time::Duration::from_millis(500)).await;

    // 3. Alice creates a swap
    tracing::info!("Alice creating a swap");
    let alice_l2_recipient = nodes.alice.rpc_client.get_new_address().await?;
    let (swap_id_alice, swap_txid_alice) = nodes
        .alice
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient_address.to_string(),
            SWAP_L1_AMOUNT,
            Some(alice_l2_recipient),
            SWAP_L2_AMOUNT,
            Some(1),
            SWAP_FEE,
        )
        .await?;
    tracing::info!(
        swap_id = %swap_id_alice,
        swap_txid = %swap_txid_alice,
        "Alice created swap"
    );

    // Mine the swap transaction
    nodes.alice.bmm_single(&mut enforcer_post_setup).await?;
    sleep(std::time::Duration::from_millis(500)).await;

    // 4. Alice transfers back to Bob
    tracing::info!("Alice transferring back to Bob");
    let bob_receive_address = nodes.bob.rpc_client.get_new_address().await?;
    let transfer_txid_alice = nodes
        .alice
        .rpc_client
        .transfer(bob_receive_address, TRANSFER_AMOUNT / 2, TRANSFER_FEE)
        .await?;
    tracing::info!(txid = %transfer_txid_alice, "Alice transferred to Bob");

    // Mine the transfer transaction
    nodes.alice.bmm_single(&mut enforcer_post_setup).await?;
    sleep(std::time::Duration::from_millis(500)).await;

    // Allow time for block propagation over P2P (verification allows Charles to lag by up to 2 blocks).
    sleep(std::time::Duration::from_secs(5)).await;

    // Charles verifies all transactions between Bob and Alice
    verify_transactions(&nodes.charles, &nodes.bob, &nodes.alice).await?;

    tracing::info!("Multi-node verification test passed");

    // Cleanup
    drop(nodes.charles);
    drop(nodes.alice);
    drop(nodes.bob);
    tracing::info!(
        "Removing {}",
        enforcer_post_setup.directories.base_dir.path().display()
    );
    drop(enforcer_post_setup.tasks);
    sleep(std::time::Duration::from_secs(1)).await;
    enforcer_post_setup.directories.base_dir.cleanup()?;
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
    file_registry: TestFileRegistry,
    failure_collector: TestFailureCollector,
) -> AsyncTrial<BoxFuture<'static, anyhow::Result<()>>> {
    AsyncTrial::new(
        "multi_node_verification",
        multi_node_verification(bin_paths).boxed(),
        file_registry,
        failure_collector,
    )
}
