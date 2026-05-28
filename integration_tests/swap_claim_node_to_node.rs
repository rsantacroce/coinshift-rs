//! Node-to-node test for the SwapClaim payout-binding fix.
//!
//! A miner node runs a full swap lifecycle (create → fill → claim → mine the
//! claim block), then a separate verifier node connects and syncs. This
//! exercises the block-validation path that now runs `validate_swap_claim`
//! (see `lib/state/block.rs::prevalidate`) on the *receiving* node: the
//! verifier must accept the legitimate claim block and converge on the same
//! chain, swap state, and sidechain wealth.
//!
//! Note on the negative (exploit) case: the malicious "pay the recipient 1
//! sat, keep the rest" claim cannot be expressed through the public RPC —
//! `claim_swap` builds the claim with the wallet, which always pays the full
//! locked value. The exploit is therefore covered at the validation layer by
//! the in-crate test `lib/state/block.rs::block_swap_claim_tests::
//! prevalidate_rejects_underpaying_claim_block`, which injects a hand-crafted
//! malicious block directly into `prevalidate`. This test is the positive,
//! end-to-end counterpart that proves honest claims still propagate and
//! validate across nodes after the fix.

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
use coinshift::types::{OutputContent, ParentChainType, SwapState};
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
const SWAP_L2_AMOUNT: u64 = 10_000_000; // 0.1 BTC
const SWAP_L1_AMOUNT: u64 = 5_000_000; // 0.05 BTC
const SWAP_FEE: u64 = 1_000;

struct Nodes {
    miner: PostSetup,
    verifier: PostSetup,
}

async fn setup(
    bin_paths: &BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<(EnforcerPostSetup, Nodes)> {
    let mut enforcer_post_setup = setup_enforcer(
        &bin_paths.others,
        Network::Regtest,
        Mode::Mempool,
        res_tx.clone(),
    )
    .await?;
    let () = propose_sidechain::<PostSetup>(&mut enforcer_post_setup).await?;
    let () = activate_sidechain::<PostSetup>(&mut enforcer_post_setup).await?;
    let () = fund_enforcer::<PostSetup>(&mut enforcer_post_setup).await?;

    let miner = PostSetup::setup(
        Init {
            coinshift_app: bin_paths.coinshift_app.clone(),
            data_dir_suffix: Some("claim-miner".to_owned()),
        },
        &enforcer_post_setup,
        res_tx.clone(),
    )
    .await?;
    tracing::info!("Setup miner node successfully");

    let verifier = PostSetup::setup(
        Init {
            coinshift_app: bin_paths.coinshift_app.clone(),
            data_dir_suffix: Some("claim-verifier".to_owned()),
        },
        &enforcer_post_setup,
        res_tx,
    )
    .await?;
    tracing::info!("Setup verifier node successfully");

    Ok((enforcer_post_setup, Nodes { miner, verifier }))
}

/// Poll until `verifier` reaches the same sidechain block count as `miner`.
async fn wait_for_sync(
    verifier: &PostSetup,
    miner: &PostSetup,
    timeout: std::time::Duration,
) -> anyhow::Result<u32> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        let v = verifier.rpc_client.getblockcount().await?;
        let m = miner.rpc_client.getblockcount().await?;
        if v == m {
            return Ok(v);
        }
        sleep(std::time::Duration::from_millis(500)).await;
    }
    let v = verifier.rpc_client.getblockcount().await?;
    let m = miner.rpc_client.getblockcount().await?;
    anyhow::bail!(
        "verifier failed to sync to miner within timeout (verifier={v}, miner={m})"
    )
}

/// Count satoshis paid to `recipient` by `Value` outputs in `list_utxos`.
async fn value_to_recipient(
    node: &PostSetup,
    recipient: coinshift::types::Address,
) -> anyhow::Result<u64> {
    let utxos = node.rpc_client.list_utxos().await?;
    let total = utxos
        .iter()
        .filter(|u| u.output.address == recipient)
        .filter_map(|u| match u.output.content {
            OutputContent::Value(v) => Some(v.to_sat()),
            _ => None,
        })
        .sum();
    Ok(total)
}

async fn swap_claim_node_to_node_task(
    bin_paths: BinPaths,
    res_tx: mpsc::UnboundedSender<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let (mut enforcer, mut nodes) = setup(&bin_paths, res_tx).await?;

    // Fund the miner.
    let miner_deposit_address = nodes.miner.get_deposit_address().await?;
    let () = deposit(
        &mut enforcer,
        &mut nodes.miner,
        &miner_deposit_address,
        DEPOSIT_AMOUNT,
        DEPOSIT_FEE,
    )
    .await?;
    tracing::info!("Deposited to miner successfully");

    // Create an open swap on the miner and include it in a block.
    let l1_recipient_address = "bcrt1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
    let (swap_id, swap_txid) = nodes
        .miner
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
    tracing::info!(%swap_id, %swap_txid, "Created open swap");
    nodes.miner.bmm_single(&mut enforcer).await?;
    sleep(std::time::Duration::from_millis(500)).await;

    // Fill the swap: declare L1 txid + the L2 claimer address. This is the
    // address the claim must (and will) pay the full locked amount to.
    let claimer_address = nodes.miner.rpc_client.get_new_address().await?;
    let fake_l1_txid_hex = "11".repeat(32);
    nodes
        .miner
        .rpc_client
        .update_swap_l1_txid(
            swap_id,
            fake_l1_txid_hex,
            1,
            Some(claimer_address),
        )
        .await?;
    sleep(std::time::Duration::from_millis(500)).await;

    let ready = nodes
        .miner
        .rpc_client
        .get_swap_status(swap_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("swap missing after fill"))?;
    anyhow::ensure!(
        matches!(ready.state, SwapState::ReadyToClaim),
        "swap should be ReadyToClaim, got {:?}",
        ready.state
    );

    // Claim and mine the claim block on the miner.
    let claim_txid = nodes.miner.rpc_client.claim_swap(swap_id, None).await?;
    tracing::info!(%swap_id, %claim_txid, "Claimed swap");
    nodes.miner.bmm_single(&mut enforcer).await?;
    sleep(std::time::Duration::from_millis(500)).await;

    // Miner: swap Completed and the recipient received the FULL locked amount.
    let completed = {
        let mut found = None;
        for _ in 0..10 {
            if let Some(s) = nodes.miner.rpc_client.get_swap_status(swap_id).await?
                && matches!(s.state, SwapState::Completed)
            {
                found = Some(s);
                break;
            }
            sleep(std::time::Duration::from_millis(200)).await;
        }
        found.ok_or_else(|| anyhow::anyhow!("swap not Completed after claim"))?
    };
    tracing::info!(state = ?completed.state, "Miner sees swap Completed");

    let miner_paid = value_to_recipient(&nodes.miner, claimer_address).await?;
    anyhow::ensure!(
        miner_paid >= SWAP_L2_AMOUNT,
        "recipient must receive the full locked amount on the miner: got {miner_paid}, expected >= {SWAP_L2_AMOUNT}"
    );

    // Connect the verifier and let it sync the chain (including the claim block).
    nodes
        .verifier
        .rpc_client
        .connect_peer(nodes.miner.net_addr().into())
        .await?;
    // A fresh block helps drive gossip/sync.
    nodes.miner.bmm_single(&mut enforcer).await?;
    let synced_height = wait_for_sync(
        &nodes.verifier,
        &nodes.miner,
        std::time::Duration::from_secs(30),
    )
    .await?;
    tracing::info!(height = synced_height, "Verifier synced to miner");

    // Verifier accepted the claim block: same wealth, swap Completed, and the
    // recipient output for the full amount is present in the verifier's state.
    let miner_wealth = nodes.miner.rpc_client.sidechain_wealth_sats().await?;
    let verifier_wealth =
        nodes.verifier.rpc_client.sidechain_wealth_sats().await?;
    anyhow::ensure!(
        miner_wealth == verifier_wealth,
        "verifier wealth ({verifier_wealth}) must equal miner wealth ({miner_wealth})"
    );

    let verifier_swap = nodes
        .verifier
        .rpc_client
        .get_swap_status(swap_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("verifier does not see the swap"))?;
    anyhow::ensure!(
        matches!(verifier_swap.state, SwapState::Completed),
        "verifier should see swap Completed, got {:?}",
        verifier_swap.state
    );

    let verifier_paid =
        value_to_recipient(&nodes.verifier, claimer_address).await?;
    anyhow::ensure!(
        verifier_paid >= SWAP_L2_AMOUNT,
        "verifier must see the full locked amount paid to the recipient: got {verifier_paid}, expected >= {SWAP_L2_AMOUNT}"
    );

    tracing::info!(
        %swap_id,
        miner_paid,
        verifier_paid,
        synced_height,
        "Node-to-node swap claim validated: honest claim accepted by both nodes"
    );

    // Cleanup.
    drop(nodes.verifier);
    drop(nodes.miner);
    drop(enforcer.tasks);
    sleep(std::time::Duration::from_secs(1)).await;
    enforcer.directories.base_dir.cleanup()?;
    Ok(())
}

async fn swap_claim_node_to_node(bin_paths: BinPaths) -> anyhow::Result<()> {
    let (res_tx, mut res_rx) = mpsc::unbounded();
    let _test_task: AbortOnDrop<()> = tokio::task::spawn({
        let res_tx = res_tx.clone();
        async move {
            let res =
                swap_claim_node_to_node_task(bin_paths, res_tx.clone()).await;
            let _send_err: Result<(), _> = res_tx.unbounded_send(res);
        }
        .in_current_span()
    })
    .into();
    res_rx.next().await.ok_or_else(|| {
        anyhow::anyhow!("Unexpected end of test task result stream")
    })?
}

pub fn swap_claim_node_to_node_trial(
    bin_paths: BinPaths,
    file_registry: TestFileRegistry,
    failure_collector: TestFailureCollector,
) -> AsyncTrial<BoxFuture<'static, anyhow::Result<()>>> {
    AsyncTrial::new(
        "swap_claim_node_to_node",
        swap_claim_node_to_node(bin_paths).boxed(),
        file_registry,
        failure_collector,
    )
}
