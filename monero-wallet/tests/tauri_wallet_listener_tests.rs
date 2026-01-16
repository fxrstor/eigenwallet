mod harness;

use anyhow::Result;
use harness::{setup_test, TestContext};
use monero_sys::TransactionInfo;
use monero_sys::WalletEventListener;
use monero_wallet::{MoneroTauriHandle, TauriWalletListener};
use serial_test::serial;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use swap_core::monero::Amount;
use tokio::time::timeout;
use tokio::sync::Notify;

struct MockTauriHandle {
    balance_updates: Arc<Mutex<Vec<(Amount, Amount)>>>,
    history_updates: Arc<Mutex<Vec<Vec<TransactionInfo>>>>,
    sync_updates: Arc<Mutex<Vec<(u64, u64, f32)>>>,
    notify: Notify,
}

impl MockTauriHandle {
    fn new() -> Self {
        Self {
            balance_updates: Arc::new(Mutex::new(Vec::new())),
            history_updates: Arc::new(Mutex::new(Vec::new())),
            sync_updates: Arc::new(Mutex::new(Vec::new())),
            notify: Notify::new(),
        }
    }

    async fn wait_for_balances_and_history(&self, dur: Duration) -> Result<()> {
        timeout(dur, async {
            loop {
                let notified = self.notify.notified();
                {
                    let balances = self.balance_updates.lock().unwrap();
                    let history = self.history_updates.lock().unwrap();
                    if !balances.is_empty() && !history.is_empty() {
                        break;
                    }
                }
                notified.await;
            }
        }).await.map_err(|_| anyhow::anyhow!("timed out waiting for balances/history"))?;
        Ok(())
    }
}

impl MoneroTauriHandle for MockTauriHandle {
    fn balance_change(&self, total_balance: Amount, unlocked_balance: Amount) {
        self.balance_updates.lock().unwrap().push((total_balance, unlocked_balance));
        self.notify.notify_waiters();
    }

    fn history_update(&self, transactions: Vec<TransactionInfo>) {
        self.history_updates.lock().unwrap().push(transactions);
        self.notify.notify_waiters();
    }

    fn sync_progress(&self, current_block: u64, target_block: u64, progress_percentage: f32) {
        self.sync_updates.lock().unwrap().push((current_block, target_block, progress_percentage));
        self.notify.notify_waiters();
    }
}

async fn make_listener(context: &TestContext) -> Result<(Arc<MockTauriHandle>, TauriWalletListener)> {
    let handle = Arc::new(MockTauriHandle::new());
    let tauri_handle = handle.clone() as Arc<dyn MoneroTauriHandle>;

    let wallets = context.create_wallets().await?;
    let wallet = wallets.main_wallet().await;

    let listener = TauriWalletListener::new(tauri_handle, wallet).await;
    Ok((handle, listener))
}

#[tokio::test]
#[serial]
async fn test_on_money_received_triggers_updates() -> Result<()> {
    setup_test(|context| async move {
        let (handle, listener) = make_listener(&context).await?;

        listener.on_money_received("txid", 1_000_000);
        handle.wait_for_balances_and_history(Duration::from_secs(5)).await?;
        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_on_money_spent_triggers_updates() -> Result<()> {
    setup_test(|context| async move {
        let (handle, listener) = make_listener(&context).await?;

        listener.on_money_spent("txid", 500_000);
        handle.wait_for_balances_and_history(Duration::from_secs(5)).await?;
        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_on_new_block_triggers_sync_progress() -> Result<()> {
    setup_test(|context| async move {
        let (handle, listener) = make_listener(&context).await?;
        listener.on_new_block(100);

        timeout(Duration::from_secs(5), async {
            loop {
                let notified = handle.notify.notified();
                {
                    let sync = handle.sync_updates.lock().unwrap();
                    if !sync.is_empty() {
                        return;
                    }
                }
                notified.await;
            }
        }).await.expect("listener didn't publish sync updates");
        Ok(())
    }).await?;
    Ok(())
}