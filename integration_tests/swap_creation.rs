//! Test swap creation functionality

use bip300301_enforcer_integration_tests::{
    integration_test::{
        activate_sidechain, deposit, fund_enforcer, propose_sidechain,
    },
    setup::{
        Mode, Network, PostSetup as EnforcerPostSetup, Sidechain as _,
        setup as setup_enforcer,
    },
    util::{AbortOnDrop, AsyncTrial},
};
use coinshift::types::{Address, GetValue, ParentChainType, SwapId, SwapState};
use coinshift_app_rpc_api::RpcClient as _;
use futures::{FutureExt as _, StreamExt as _, channel::mpsc, future::BoxFuture};
use tokio::time::sleep;
use tracing::Instrument as _;

use crate::{
    setup::{Init, PostSetup},
    util::BinPaths,
};

/// Initial setup for the test
async fn setup(
    bin_paths: &BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<EnforcerPostSetup> {
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
    Ok(enforcer_post_setup)
}

const DEPOSIT_AMOUNT: bitcoin::Amount = bitcoin::Amount::from_sat(21_000_000);
const DEPOSIT_FEE: bitcoin::Amount = bitcoin::Amount::from_sat(1_000_000);
const SWAP_L2_AMOUNT: u64 = 10_000_000; // 0.1 BTC
const SWAP_L1_AMOUNT: u64 = 5_000_000; // 0.05 BTC
const SWAP_FEE: u64 = 1_000; // 0.00001 BTC

/// Verify that a swap was created successfully
async fn verify_swap_created(
    rpc_client: &jsonrpsee::http_client::HttpClient,
    swap_id: SwapId,
    expected_l2_amount: u64,
    expected_l1_amount: u64,
    expected_l2_recipient: Option<Address>,
) -> anyhow::Result<()> {
    let swap = rpc_client
        .get_swap_status(swap_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Swap not found"))?;

    // Verify swap details
    anyhow::ensure!(
        swap.id == swap_id,
        "Swap ID mismatch: expected {:?}, got {:?}",
        swap_id,
        swap.id
    );
    anyhow::ensure!(
        swap.l2_amount.to_sat() == expected_l2_amount,
        "L2 amount mismatch: expected {}, got {}",
        expected_l2_amount,
        swap.l2_amount.to_sat()
    );
    anyhow::ensure!(
        swap.l1_amount.map(|a| a.to_sat()) == Some(expected_l1_amount),
        "L1 amount mismatch: expected {}, got {:?}",
        expected_l1_amount,
        swap.l1_amount.map(|a| a.to_sat())
    );
    anyhow::ensure!(
        swap.l2_recipient == expected_l2_recipient,
        "L2 recipient mismatch: expected {:?}, got {:?}",
        expected_l2_recipient,
        swap.l2_recipient
    );
    anyhow::ensure!(
        matches!(swap.state, SwapState::Pending),
        "Swap state should be Pending, got {:?}",
        swap.state
    );

    tracing::info!("Swap created successfully: {:?}", swap);
    Ok(())
}

/// Wait for swap transaction to be included in a block
async fn wait_for_swap_in_block(
    sidechain: &mut PostSetup,
    enforcer: &mut EnforcerPostSetup,
    swap_txid: coinshift::types::Txid,
    swap_id: SwapId,
) -> anyhow::Result<()> {
    // BMM a block to include the swap transaction
    tracing::debug!(
        swap_id = %swap_id,
        swap_txid = %swap_txid,
        "BMM 1 block to include swap transaction"
    );
    sidechain.bmm_single(enforcer).await?;

    // Verify swap is accessible after being included in block
    // Swaps are only saved to database when included in a block
    let swaps = sidechain.rpc_client.list_swaps().await?;
    anyhow::ensure!(
        swaps.iter().any(|s| s.id == swap_id),
        "Swap {} not found in list_swaps after block inclusion",
        swap_id
    );

    tracing::info!(
        swap_id = %swap_id,
        "Swap transaction included in block and persisted"
    );
    Ok(())
}

/// Verify that UTXOs are locked for the swap
async fn verify_swap_locks_utxos(
    rpc_client: &jsonrpsee::http_client::HttpClient,
    swap_id: SwapId,
    expected_locked_amount: u64,
) -> anyhow::Result<()> {
    let utxos = rpc_client.list_utxos().await?;
    
    // Find locked outputs for this swap and calculate total locked amount
    let mut total_locked: u64 = 0;
    let mut has_locked_outputs = false;
    
    for utxo in &utxos {
        if let coinshift::types::OutputContent::SwapPending { value, swap_id: locked_swap_id } = &utxo.output.content {
            if *locked_swap_id == swap_id.0 {
                has_locked_outputs = true;
                total_locked += value.to_sat();
            }
        }
    }

    anyhow::ensure!(
        has_locked_outputs,
        "No locked outputs found for swap {}",
        swap_id
    );
    
    anyhow::ensure!(
        total_locked == expected_locked_amount,
        "Locked amount mismatch: expected {}, got {}",
        expected_locked_amount,
        total_locked
    );

    tracing::info!(
        "Verified swap locks UTXOs: {} sats locked for swap {}",
        total_locked,
        swap_id
    );
    Ok(())
}

/// Wait (with retries) for UTXOs to be locked for the swap
async fn wait_for_locked_utxos(
    rpc_client: &jsonrpsee::http_client::HttpClient,
    swap_id: SwapId,
    expected_locked_amount: u64,
) -> anyhow::Result<()> {
    const MAX_RETRIES: usize = 10;
    const RETRY_DELAY_MS: u64 = 200;

    for attempt in 0..MAX_RETRIES {
        let res =
            verify_swap_locks_utxos(rpc_client, swap_id, expected_locked_amount).await;
        if res.is_ok() {
            return Ok(());
        }
        tracing::debug!(
            attempt,
            swap_id = %swap_id,
            "Locked UTXOs not yet visible, retrying..."
        );
        sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
    }

    // Final attempt, propagate error
    verify_swap_locks_utxos(rpc_client, swap_id, expected_locked_amount).await
}

async fn setup_swapper(
    bin_paths: &BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
    data_dir_suffix: &str,
) -> anyhow::Result<(PostSetup, EnforcerPostSetup)> {
    let enforcer_post_setup = setup(bin_paths, res_tx.clone()).await?;

    let sidechain = PostSetup::setup(
        Init {
            coinshift_app: bin_paths.coinshift_app.clone(),
            data_dir_suffix: Some(data_dir_suffix.to_owned()),
        },
        &enforcer_post_setup,
        res_tx,
    )
    .await?;
    tracing::info!(
        "Setup Coinshift swapper node successfully (suffix={})",
        data_dir_suffix
    );

    Ok((sidechain, enforcer_post_setup))
}

async fn cleanup_swapper(
    sidechain: PostSetup,
    enforcer_post_setup: EnforcerPostSetup,
) -> anyhow::Result<()> {
    drop(sidechain);
    tracing::info!("Removing {}", enforcer_post_setup.out_dir.path().display());
    drop(enforcer_post_setup.tasks);
    // Wait for tasks to die
    sleep(std::time::Duration::from_secs(1)).await;
    enforcer_post_setup.out_dir.cleanup()?;
    Ok(())
}

async fn swap_creation_fixed_task(
    bin_paths: BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let (mut sidechain, mut enforcer_post_setup) =
        setup_swapper(&bin_paths, res_tx.clone(), "swapper-fixed").await?;

    // Get deposit address and deposit funds
    let deposit_address = sidechain.get_deposit_address().await?;
    let () = deposit(
        &mut enforcer_post_setup,
        &mut sidechain,
        &deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Deposited to sidechain successfully");

    // Get a new address for L2 recipient (pre-specified swap)
    let l2_recipient_address = sidechain.rpc_client.get_new_address().await?;
    
    // Generate a regtest address for L1 recipient
    let l1_recipient_address = "bcrt1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
    
    // Create a pre-specified swap (with l2_recipient)
    tracing::info!("Creating pre-specified swap");
    let (swap_id, swap_txid) = sidechain
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient_address.to_string(),
            SWAP_L1_AMOUNT,
            Some(l2_recipient_address),
            SWAP_L2_AMOUNT,
            Some(1), // required_confirmations
            SWAP_FEE,
        )
        .await?;
    tracing::info!(
        swap_id = %swap_id,
        swap_txid = %swap_txid,
        "Created pre-specified swap transaction"
    );

    // Wait for swap to be included in block (swaps are only saved when included in a block)
    wait_for_swap_in_block(&mut sidechain, &mut enforcer_post_setup, swap_txid, swap_id)
        .await?;
    
    // Wait for wallet update task to sync state changes (locked outputs, spent UTXOs)
    // This ensures the wallet's view is current before proceeding
    sleep(std::time::Duration::from_millis(500)).await;

    // Now verify swap was created and persisted (after block inclusion)
    verify_swap_created(
        &sidechain.rpc_client,
        swap_id,
        SWAP_L2_AMOUNT,
        SWAP_L1_AMOUNT,
        Some(l2_recipient_address),
    )
    .await?;

    // Verify UTXOs are locked
    verify_swap_locks_utxos(&sidechain.rpc_client, swap_id, SWAP_L2_AMOUNT)
        .await?;

    // Verify list_swaps and list_swaps_by_recipient contain the swap
    let all_swaps = sidechain.rpc_client.list_swaps().await?;
    anyhow::ensure!(
        all_swaps.iter().any(|s| s.id == swap_id),
        "Pre-specified swap not found in list_swaps"
    );
    let recipient_swaps = sidechain
        .rpc_client
        .list_swaps_by_recipient(l2_recipient_address)
        .await?;
    anyhow::ensure!(
        recipient_swaps.iter().any(|s| s.id == swap_id),
        "Pre-specified swap not found in list_swaps_by_recipient"
    );

    tracing::info!("Fixed swap creation test passed");

    cleanup_swapper(sidechain, enforcer_post_setup).await
}

async fn swap_creation_open_task(
    bin_paths: BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let (mut sidechain, mut enforcer_post_setup) =
        setup_swapper(&bin_paths, res_tx.clone(), "swapper-open").await?;

    // Get deposit address and deposit funds
    let deposit_address = sidechain.get_deposit_address().await?;
    let () = deposit(
        &mut enforcer_post_setup,
        &mut sidechain,
        &deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Deposited to sidechain successfully");

    let l1_recipient_address = "bcrt1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";

    // Create an open swap (without l2_recipient)
    tracing::info!("Creating open swap");
    let (swap_id, swap_txid) = sidechain
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient_address.to_string(),
            SWAP_L1_AMOUNT,
            None, // None = open swap
            SWAP_L2_AMOUNT,
            Some(1),
            SWAP_FEE,
        )
        .await?;
    tracing::info!(
        swap_id = %swap_id,
        swap_txid = %swap_txid,
        "Created open swap transaction"
    );

    // Wait for open swap to be included in block (swaps are only saved when included in a block)
    wait_for_swap_in_block(
        &mut sidechain,
        &mut enforcer_post_setup,
        swap_txid,
        swap_id,
    )
    .await?;

    // Wait for wallet update task to sync state changes (locked outputs, spent UTXOs)
    sleep(std::time::Duration::from_millis(500)).await;

    // Now verify open swap was created and persisted (after block inclusion)
    verify_swap_created(
        &sidechain.rpc_client,
        swap_id,
        SWAP_L2_AMOUNT,
        SWAP_L1_AMOUNT,
        None, // Open swap has no l2_recipient
    )
    .await?;
    
    // Verify open swap locks UTXOs as expected
    verify_swap_locks_utxos(&sidechain.rpc_client, swap_id, SWAP_L2_AMOUNT)
        .await?;

    // Verify open swap has no l2_recipient and is listed
    let open_swap = sidechain
        .rpc_client
        .get_swap_status(swap_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Open swap not found after block inclusion"))?;
    anyhow::ensure!(
        open_swap.l2_recipient.is_none(),
        "Open swap should have no l2_recipient"
    );

    let all_swaps = sidechain.rpc_client.list_swaps().await?;
    anyhow::ensure!(
        all_swaps.iter().any(|s| s.id == swap_id),
        "Open swap not found in list_swaps"
    );

    tracing::info!("Open swap creation test passed");

    cleanup_swapper(sidechain, enforcer_post_setup).await
}

async fn swap_creation_open_fill_task(
    bin_paths: BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let (mut sidechain, mut enforcer_post_setup) =
        setup_swapper(&bin_paths, res_tx.clone(), "swapper-open-fill").await?;

    // Fund the wallet
    let deposit_address = sidechain.get_deposit_address().await?;
    deposit(
        &mut enforcer_post_setup,
        &mut sidechain,
        &deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Deposited to sidechain successfully");

    let l1_recipient_address = "bcrt1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";

    // Create an open swap (without l2_recipient)
    tracing::info!("Creating open swap to later fill");
    let (swap_id, swap_txid) = sidechain
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient_address.to_string(),
            SWAP_L1_AMOUNT,
            None, // open swap
            SWAP_L2_AMOUNT,
            Some(1),
            SWAP_FEE,
        )
        .await?;
    tracing::info!(
        swap_id = %swap_id,
        swap_txid = %swap_txid,
        "Created open swap transaction"
    );

    // Include the swap create tx in a block
    wait_for_swap_in_block(
        &mut sidechain,
        &mut enforcer_post_setup,
        swap_txid,
        swap_id,
    )
    .await?;
    sleep(std::time::Duration::from_millis(500)).await;

    // Ensure it exists and is pending with locked UTXOs
    verify_swap_created(
        &sidechain.rpc_client,
        swap_id,
        SWAP_L2_AMOUNT,
        SWAP_L1_AMOUNT,
        None,
    )
    .await?;
    wait_for_locked_utxos(&sidechain.rpc_client, swap_id, SWAP_L2_AMOUNT).await?;

    // Simulate detecting the L1 tx: mark as confirmed so swap moves to ReadyToClaim
    let fake_l1_txid_hex = "11".repeat(32);
    sidechain
        .rpc_client
        .update_swap_l1_txid(swap_id, fake_l1_txid_hex.clone(), 1)
        .await?;
    // Allow wallet/state tasks to catch up
    sleep(std::time::Duration::from_millis(500)).await;
    wait_for_locked_utxos(&sidechain.rpc_client, swap_id, SWAP_L2_AMOUNT).await?;

    tracing::info!(
        "Open swap state updated to ReadyToClaim after L1 TXID update. swap_id={}, fake_l1_txid={}",
        swap_id,
        fake_l1_txid_hex
    );
    let status_ready = sidechain
        .rpc_client
        .get_swap_status(swap_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Swap not found after L1 update"))?;
    anyhow::ensure!(
        matches!(status_ready.state, SwapState::ReadyToClaim),
        "Swap not ReadyToClaim after L1 update: {:?}",
        status_ready.state
    );

    // Claim the swap as a specific L2 address
    let claimer_address = sidechain.rpc_client.get_new_address().await?;
    let claim_txid = sidechain
        .rpc_client
        .claim_swap(swap_id, Some(claimer_address))
        .await?;
    tracing::info!(swap_id = %swap_id, claim_txid = %claim_txid, "Claimed swap");

    // Mine the claim transaction into a block
    sidechain.bmm_single(&mut enforcer_post_setup).await?;
    sleep(std::time::Duration::from_millis(500)).await;

    // Verify completion
    // Wait until the swap is marked completed
    const MAX_STATUS_RETRIES: usize = 10;
    const STATUS_DELAY_MS: u64 = 200;
    let mut completed = None;
    for _ in 0..MAX_STATUS_RETRIES {
        let status = sidechain.rpc_client.get_swap_status(swap_id).await?;
        if let Some(s) = status {
            if matches!(s.state, SwapState::Completed) {
                completed = Some(s);
                break;
            }
        }
        sleep(std::time::Duration::from_millis(STATUS_DELAY_MS)).await;
    }
    let completed_swap = completed
        .ok_or_else(|| anyhow::anyhow!("Swap not marked Completed after claim"))?;

    // Locked outputs should be released
    let utxos_after = sidechain.rpc_client.list_utxos().await?;
    let still_locked = utxos_after.iter().any(|utxo| {
        matches!(
            utxo.output.content,
            coinshift::types::OutputContent::SwapPending { swap_id: locked, .. }
                if locked == swap_id.0
        )
    });
    anyhow::ensure!(
        !still_locked,
        "Expected no locked outputs after swap completion"
    );

    // Final report
    tracing::info!(
        swap_id = %swap_id,
        swap_create_txid = %swap_txid,
        fake_l1_txid_hex = %fake_l1_txid_hex,
        claim_txid = %claim_txid,
        l1_recipient = l1_recipient_address,
        l1_amount_sats = SWAP_L1_AMOUNT,
        l2_amount_sats = SWAP_L2_AMOUNT,
        claimer_address = %claimer_address,
        final_state = ?completed_swap.state,
        utxos_total = utxos_after.len(),
        "Open swap fill report: swap completed and locks released"
    );

    tracing::info!("Open swap fill and claim test passed");

    cleanup_swapper(sidechain, enforcer_post_setup).await
}

async fn swap_creation_fixed(bin_paths: BinPaths) -> anyhow::Result<()> {
    let (res_tx, mut res_rx) = mpsc::unbounded();
    let _test_task: AbortOnDrop<()> = tokio::task::spawn({
        let res_tx = res_tx.clone();
        async move {
            let res = swap_creation_fixed_task(bin_paths, res_tx.clone()).await;
            let _send_err: Result<(), _> = res_tx.unbounded_send(res);
        }
        .in_current_span()
    })
    .into();
    res_rx.next().await.ok_or_else(|| {
        anyhow::anyhow!("Unexpected end of test task result stream")
    })?
}

async fn swap_creation_open(bin_paths: BinPaths) -> anyhow::Result<()> {
    let (res_tx, mut res_rx) = mpsc::unbounded();
    let _test_task: AbortOnDrop<()> = tokio::task::spawn({
        let res_tx = res_tx.clone();
        async move {
            let res = swap_creation_open_task(bin_paths, res_tx.clone()).await;
            let _send_err: Result<(), _> = res_tx.unbounded_send(res);
        }
        .in_current_span()
    })
    .into();
    res_rx.next().await.ok_or_else(|| {
        anyhow::anyhow!("Unexpected end of test task result stream")
    })?
}

async fn swap_creation_open_fill(bin_paths: BinPaths) -> anyhow::Result<()> {
    let (res_tx, mut res_rx) = mpsc::unbounded();
    let _test_task: AbortOnDrop<()> = tokio::task::spawn({
        let res_tx = res_tx.clone();
        async move {
            let res = swap_creation_open_fill_task(bin_paths, res_tx.clone()).await;
            let _send_err: Result<(), _> = res_tx.unbounded_send(res);
        }
        .in_current_span()
    })
    .into();
    res_rx.next().await.ok_or_else(|| {
        anyhow::anyhow!("Unexpected end of test task result stream")
    })?
}

pub fn swap_creation_fixed_trial(
    bin_paths: BinPaths,
) -> AsyncTrial<BoxFuture<'static, anyhow::Result<()>>> {
    AsyncTrial::new("swap_creation_fixed", swap_creation_fixed(bin_paths).boxed())
}

pub fn swap_creation_open_trial(
    bin_paths: BinPaths,
) -> AsyncTrial<BoxFuture<'static, anyhow::Result<()>>> {
    AsyncTrial::new("swap_creation_open", swap_creation_open(bin_paths).boxed())
}

pub fn swap_creation_open_fill_trial(
    bin_paths: BinPaths,
) -> AsyncTrial<BoxFuture<'static, anyhow::Result<()>>> {
    AsyncTrial::new(
        "swap_creation_open_fill",
        swap_creation_open_fill(bin_paths).boxed(),
    )
}

/// Test that swaps with same parameters but different sender addresses are allowed
/// Note: Because swap_id includes the l2_sender_address, swaps with the same parameters
/// but different sender addresses will have different swap_ids and are treated as different swaps.
async fn swap_creation_duplicate_task(
    bin_paths: BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let (mut sidechain, mut enforcer_post_setup) =
        setup_swapper(&bin_paths, res_tx.clone(), "swapper-duplicate").await?;

    // Get deposit address and deposit funds
    let deposit_address = sidechain.get_deposit_address().await?;
    let () = deposit(
        &mut enforcer_post_setup,
        &mut sidechain,
        &deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Deposited to sidechain successfully");

    // Get addresses for swap
    let l2_recipient_address = sidechain.rpc_client.get_new_address().await?;
    let l1_recipient_address = "bcrt1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";

    // Create first swap
    tracing::info!("Creating first swap");
    let (swap_id, swap_txid) = sidechain
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient_address.to_string(),
            SWAP_L1_AMOUNT,
            Some(l2_recipient_address),
            SWAP_L2_AMOUNT,
            Some(1),
            SWAP_FEE,
        )
        .await?;
    tracing::info!(
        swap_id = %swap_id,
        swap_txid = %swap_txid,
        "Created first swap transaction"
    );

    // Wait for swap to be included in block
    wait_for_swap_in_block(&mut sidechain, &mut enforcer_post_setup, swap_txid, swap_id)
        .await?;
    sleep(std::time::Duration::from_millis(500)).await;

    // Verify first swap was created
    verify_swap_created(
        &sidechain.rpc_client,
        swap_id,
        SWAP_L2_AMOUNT,
        SWAP_L1_AMOUNT,
        Some(l2_recipient_address),
    )
    .await?;

    // Try to create a swap with the same parameters again
    // Note: Because swap_id includes the l2_sender_address (from the first input UTXO),
    // and the first swap already spent some UTXOs, the second swap will use different
    // UTXOs with a different sender address, resulting in a different swap_id.
    // This means they are technically different swaps, not duplicates.
    // 
    // To test true duplicate detection, we would need to create a swap with the exact
    // same swap_id, which requires using the same sender address. However, after the
    // first swap, that address's UTXOs may have been spent.
    //
    // For now, we verify that creating a swap with similar parameters (but different
    // sender address) is allowed, as they have different swap_ids.
    tracing::info!("Attempting to create another swap with same parameters (different sender address)");
    let second_swap_result = sidechain
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient_address.to_string(),
            SWAP_L1_AMOUNT,
            Some(l2_recipient_address),
            SWAP_L2_AMOUNT,
            Some(1),
            SWAP_FEE,
        )
        .await?;

    let (second_swap_id, second_swap_txid) = second_swap_result;
    tracing::info!(
        second_swap_id = %second_swap_id,
        second_swap_txid = %second_swap_txid,
        "Second swap transaction created with different swap_id (different sender address)"
    );

    // Verify the second swap has a different swap_id (because sender address is different)
    anyhow::ensure!(
        second_swap_id != swap_id,
        "Second swap should have different swap_id due to different sender address"
    );

    // Include the second swap in a block - it should succeed as it's a different swap
    wait_for_swap_in_block(&mut sidechain, &mut enforcer_post_setup, second_swap_txid, second_swap_id)
        .await?;
    sleep(std::time::Duration::from_millis(500)).await;
    
    // Verify both swaps exist
    let all_swaps = sidechain.rpc_client.list_swaps().await?;
    let original_found = all_swaps.iter().any(|s| s.id == swap_id);
    let second_found = all_swaps.iter().any(|s| s.id == second_swap_id);
    
    anyhow::ensure!(
        original_found,
        "Original swap not found after second swap creation"
    );
    anyhow::ensure!(
        second_found,
        "Second swap not found after block inclusion"
    );
    
    // Verify both swaps exist (they have different swap_ids so both are valid)
    anyhow::ensure!(
        all_swaps.len() == 2,
        "Expected exactly 2 swaps, found {}",
        all_swaps.len()
    );
    
    tracing::info!("Both swaps created successfully (different swap_ids due to different sender addresses)");

    tracing::info!("Duplicate swap creation test passed");

    cleanup_swapper(sidechain, enforcer_post_setup).await
}

/// Test that swap creation with insufficient funds fails
async fn swap_creation_insufficient_funds_task(
    bin_paths: BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let (mut sidechain, mut enforcer_post_setup) =
        setup_swapper(&bin_paths, res_tx.clone(), "swapper-insufficient").await?;

    // Get deposit address and deposit funds
    let deposit_address = sidechain.get_deposit_address().await?;
    let () = deposit(
        &mut enforcer_post_setup,
        &mut sidechain,
        &deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Deposited to sidechain successfully");

    // Check available balance
    let utxos = sidechain.rpc_client.list_utxos().await?;
    let total_balance: u64 = utxos
        .iter()
        .map(|utxo| utxo.output.get_value().to_sat())
        .sum();
    tracing::info!("Total balance: {} sats", total_balance);

    // Try to create a swap with more than available balance
    let excessive_amount = total_balance + 1_000_000; // More than available
    let l2_recipient_address = sidechain.rpc_client.get_new_address().await?;
    let l1_recipient_address = "bcrt1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";

    tracing::info!(
        "Attempting to create swap with {} sats (more than available {})",
        excessive_amount,
        total_balance
    );

    let result = sidechain
        .rpc_client
        .create_swap(
            ParentChainType::Regtest,
            l1_recipient_address.to_string(),
            SWAP_L1_AMOUNT,
            Some(l2_recipient_address),
            excessive_amount,
            Some(1),
            SWAP_FEE,
        )
        .await;

    // This should fail with insufficient funds error
    anyhow::ensure!(
        result.is_err(),
        "Swap creation with insufficient funds should have failed"
    );

    let error_msg = format!("{:#}", result.unwrap_err());
    tracing::info!("Insufficient funds error (expected): {}", error_msg);

    // Verify no swap was created
    let swaps = sidechain.rpc_client.list_swaps().await?;
    anyhow::ensure!(
        swaps.is_empty(),
        "No swaps should exist after failed creation"
    );

    tracing::info!("Insufficient funds validation test passed");

    cleanup_swapper(sidechain, enforcer_post_setup).await
}

async fn swap_creation_duplicate(bin_paths: BinPaths) -> anyhow::Result<()> {
    let (res_tx, mut res_rx) = mpsc::unbounded();
    let _test_task: AbortOnDrop<()> = tokio::task::spawn({
        let res_tx = res_tx.clone();
        async move {
            let res = swap_creation_duplicate_task(bin_paths, res_tx.clone()).await;
            let _send_err: Result<(), _> = res_tx.unbounded_send(res);
        }
        .in_current_span()
    })
    .into();
    res_rx.next().await.ok_or_else(|| {
        anyhow::anyhow!("Unexpected end of test task result stream")
    })?
}

async fn swap_creation_insufficient_funds(bin_paths: BinPaths) -> anyhow::Result<()> {
    let (res_tx, mut res_rx) = mpsc::unbounded();
    let _test_task: AbortOnDrop<()> = tokio::task::spawn({
        let res_tx = res_tx.clone();
        async move {
            let res = swap_creation_insufficient_funds_task(bin_paths, res_tx.clone()).await;
            let _send_err: Result<(), _> = res_tx.unbounded_send(res);
        }
        .in_current_span()
    })
    .into();
    res_rx.next().await.ok_or_else(|| {
        anyhow::anyhow!("Unexpected end of test task result stream")
    })?
}

pub fn swap_creation_duplicate_trial(
    bin_paths: BinPaths,
) -> AsyncTrial<BoxFuture<'static, anyhow::Result<()>>> {
    AsyncTrial::new(
        "swap_creation_duplicate",
        swap_creation_duplicate(bin_paths).boxed(),
    )
}

pub fn swap_creation_insufficient_funds_trial(
    bin_paths: BinPaths,
) -> AsyncTrial<BoxFuture<'static, anyhow::Result<()>>> {
    AsyncTrial::new(
        "swap_creation_insufficient_funds",
        swap_creation_insufficient_funds(bin_paths).boxed(),
    )
}

