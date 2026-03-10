mod harness;

use anyhow::Result;
use harness::{setup_test, WALLET_NAME};
use monero_address::{AddressType, Network};
use monero_oxide_ext::{PublicKey, PrivateKey};
use monero_sys::TransactionInfo;
use monero_wallet::{MoneroTauriHandle, Wallets};
use serial_test::serial;
use std::sync::{Arc, Mutex};
use swap_core::monero::Amount;
use uuid::Uuid;
use std::time::Duration;

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

#[tokio::test]
#[serial]
async fn test_tauri_listener() -> Result<()> {
    setup_test(|context| async move {
        let handle = Arc::new(RecordingTauriHandle::new());
        let tauri_handle = Some(handle.clone() as Arc<dyn MoneroTauriHandle>);

        let wallets = Wallets::new(
            context.wallet_dir.path().to_path_buf(),
            WALLET_NAME.to_string(),
            context.daemon.clone(),
            Network::Mainnet,
            true,
            tauri_handle,
            None,
        ).await?;

        let main_wallet = wallets.main_wallet().await;
        
        context.sync_wallet(&main_wallet).await?;

        assert!(!handle.sync_updates.lock().unwrap().is_empty());

        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_change_monero_node() -> Result<()> {
    setup_test(|context| async move {
        let wallets = context.create_wallets().await?;
        let main_wallet = wallets.main_wallet().await;

        let initial_height = main_wallet.blockchain_height().await?;

        let same_daemon = context.daemon.clone();
        wallets.change_monero_node(same_daemon).await?;

        let height_after = main_wallet.blockchain_height().await?;
        assert!(height_after >= initial_height);

        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_recent_wallets() -> Result<()> {
    setup_test(|context| async move {
        let db_dir = tempfile::TempDir::new()?;
        let db = Arc::new(monero_sys::Database::new(db_dir.path().to_path_buf()).await?);

        let wallets = Wallets::new(
            context.wallet_dir.path().to_path_buf(),
            WALLET_NAME.to_string(),
            context.daemon.clone(),
            Network::Mainnet,
            true,
            None,
            Some(db.clone()),
        ).await?;

        let recent = wallets.get_recent_wallets().await?;
        assert!(!recent.is_empty());
        assert!(recent.iter().any(|p| p.contains(WALLET_NAME)));

        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_swap_wallet() -> Result<()> {
    setup_test(|context| async move {
        use swap_core::monero::primitives::{PrivateViewKey, TxHash};
        let (spend_key, view_key, address) = {
            let mut rng = rand::thread_rng();

            let spend_key = PrivateKey::from_scalar(swap_core::monero::Scalar::random(&mut rng));
            let view_key = PrivateViewKey::new_random(&mut rng);

            let address = {
                let public_spend_key = PublicKey::from_private_key(&spend_key);
                let public_view_key = PublicKey::from_private_key(&view_key.into());

                monero_address::MoneroAddress::new(
                    Network::Mainnet,
                    AddressType::Subaddress,
                    public_spend_key.decompress(),
                    public_view_key.decompress(),
                )
            };

            (spend_key, view_key, address)
        };

        let amount = 1_000_000_000_000u64; // 1 XMR
        let tx = TxHash(context.monero.wallet("miner")?.transfer(&address, amount).await?.txid,);
        context.generate_blocks(12).await?;

        let wallets = context.create_wallets().await?;
        let swap_id = Uuid::new_v4();
        let swap_wallet_arc = wallets
            .swap_wallet_spendable(swap_id, spend_key.clone(), view_key.clone(), tx.clone())
            .await
            .expect("swap wallet created");

        let swap_wallet = swap_wallet_arc.clone();
        swap_wallet.set_restore_height(0).await?;
        swap_wallet.scan_transaction(tx.0.clone()).await?;
        swap_wallet.refresh_blocking().await?;
        context.wait_for_unlocked_balance(&swap_wallet, amount, 120).await?;
        let total = swap_wallet.total_balance().await?.as_pico();
        assert!(total >= amount, "swap wallet balance {} < expected {}", total, amount);
        
        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_change_monero_node_to_different_daemon() -> Result<()> {
    use monero_harness::{image, Monero, Cli};

    setup_test(|context| async move {
        let wallets = context.create_wallets().await?;
        let main_wallet = wallets.main_wallet().await;

        context.generate_blocks(12).await?;
        context.sync_wallet(&main_wallet).await?;
        let height_a = main_wallet.blockchain_height().await?;

        let cli = Cli::default();
        let wallet_name = "secondary_wallet".to_string();
        let wallet_name_static: &'static str = Box::leak(wallet_name.into_boxed_str());
        let (_monero_b, monerod_b, _wallet_b) = Monero::new(&cli, vec![wallet_name_static]).await?;
        let monerod_b_port = monerod_b.ports().map_to_host_port_ipv4(image::RPC_PORT).ok_or_else(|| anyhow::anyhow!("failed to map monerod RPC port"))?;
        let daemon_b = monero_sys::Daemon {
            hostname: "127.0.0.1".to_string(),
            port: monerod_b_port,
            ssl: false,
        };

        wallets.change_monero_node(daemon_b.clone()).await?;
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let h = main_wallet.blockchain_height().await?;
                if h < height_a {
                    break Ok::<(), anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }).await.map_err(|_| anyhow::anyhow!("wallet did not rebind to new daemon"))??;
        let height_b = main_wallet.blockchain_height().await?;
        assert!(height_b < height_a, "wallet did not switch to new daemon: height_a={}, height_b={}", height_a, height_b);
        Ok(())
    }).await?;
    Ok(())
}
