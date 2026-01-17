mod harness;

use anyhow::Result;
use harness::{setup_test, TestContext};
use monero_sys::{TransactionInfo, WalletEventListener};
use monero_wallet::{MoneroTauriHandle, TauriWalletListener, Wallets};
use serial_test::serial;
use std::sync::{Arc, Mutex};
use swap_core::monero::Amount;

#[derive(Default)]
struct RecordingTauriHandle {
    balance_updates: Mutex<Vec<(Amount, Amount)>>,
    history_updates: Mutex<Vec<Vec<TransactionInfo>>> ,
    sync_updates: Mutex<Vec<(u64, u64, f32)>>,
}

impl RecordingTauriHandle {
    fn new() -> Self { Default::default() }
}

impl MoneroTauriHandle for RecordingTauriHandle {
    fn balance_change(&self, total_balance: Amount, unlocked_balance: Amount) {
        self.balance_updates.lock().unwrap().push((total_balance, unlocked_balance));
    }

    fn history_update(&self, transactions: Vec<TransactionInfo>) {
        self.history_updates.lock().unwrap().push(transactions);
    }

    fn sync_progress(&self, current_block: u64, target_block: u64, progress_percentage: f32) {
        self.sync_updates.lock().unwrap().push((current_block, target_block, progress_percentage));
    }
}

async fn wait_until<F>(timeout_ms: u64, mut f: F) -> anyhow::Result<()>
where
    F: FnMut() -> bool,
{
    let start = std::time::Instant::now();
    loop {
        if f() {
            return Ok(());
        }
        if start.elapsed().as_millis() > timeout_ms as u128 {
            anyhow::bail!("timeout waiting for condition");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn make_listener(
    context: &TestContext,
) -> Result<(Arc<RecordingTauriHandle>, TauriWalletListener, Wallets, Arc<monero_sys::WalletHandle>)> {
    let handle = Arc::new(RecordingTauriHandle::new());
    let tauri_handle = handle.clone() as Arc<dyn MoneroTauriHandle>;

    let wallets = context.create_wallets().await?;
    let wallet = wallets.main_wallet().await;
    let wallet_arc = Arc::new(wallet);

    let listener = TauriWalletListener::new(tauri_handle.clone(), wallet_arc.clone()).await;
    Ok((handle, listener, wallets, wallet_arc))
}

#[tokio::test]
#[serial]
async fn test_on_money_received_triggers_updates() -> Result<()> {
    setup_test(|context| async move{
        let (handle, listener, _wallets, _wallet) = make_listener(&context).await?;

        listener.on_money_received("txid", 1_000_000);

        wait_until(5_000, || {
            !handle.balance_updates.lock().unwrap().is_empty()
        }).await?;

        wait_until(5_000, || {
            !handle.history_updates.lock().unwrap().is_empty()
        }).await?;

        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_on_money_spent_triggers_updates() -> Result<()> {
    setup_test(|context| async move {
        let (handle, listener, _wallets, _wallet) = make_listener(&context).await?;

        listener.on_money_spent("txid", 500_000);

        wait_until(5_000, || {
            !handle.balance_updates.lock().unwrap().is_empty()
        }).await?;

        wait_until(5_000, || {
            !handle.history_updates.lock().unwrap().is_empty()
        }).await?;

        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_on_new_block_triggers_sync_progress() -> Result<()> {
    setup_test(|context| async move {
        let (handle, listener, _wallets, _wallet) = make_listener(&context).await?;
        listener.on_new_block(100);

        wait_until(5_000, || {
            !handle.sync_updates.lock().unwrap().is_empty()
        }).await?;

        Ok(())
    }).await?;
    Ok(())
}
