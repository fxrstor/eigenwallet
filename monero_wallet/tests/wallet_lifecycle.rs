mod harness;

use anyhow::{Context, Result};
use harness::{setup_test, WALLET_NAME};
use monero_address::Network;
use monero_harness::{image, Monero};
use monero_sys::{Daemon, WalletHandle};
use monero_wallet::Wallets;
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
#[serial]
async fn test_creates_new_wallet() -> Result<()> {
    setup_test(|ctx| async move {
        let wallet = ctx.open_regtest_wallet(ctx.wallet_path()).await?;
        let address = wallet.main_address().await?;

        assert_eq!(address.network(), Network::Mainnet);
        assert!(
            address.to_string().starts_with('4'),
            "unexpected address prefix: {address}"
        );

        Ok(())
    })
    .await
}

#[tokio::test]
#[serial]
async fn test_reopens_existing_wallet() -> Result<()> {
    setup_test(|ctx| async move {
        let wallet_path = ctx.wallet_path();

        let initial_address = {
            let wallet = ctx.open_regtest_wallet(ctx.wallet_path()).await?;
            wallet.main_address().await?
        };

        let reopened = ctx
            .open_regtest_wallet(wallet_path)
            .await
            .context("re-opening existing wallet")?;

        let address = reopened.main_address().await?;
        assert_eq!(address.network(), Network::Mainnet);
        assert_eq!(address.to_string(), initial_address.to_string());

        Ok(())
    })
    .await
}

#[tokio::test]
#[serial]
async fn test_restores_wallet_from_seed() -> Result<()> {
    setup_test(|ctx| async move {
        let (seed, original_address) = {
            let wallet = ctx.open_regtest_wallet(ctx.wallet_path()).await?;
            let seed = wallet.seed().await?;
            let address = wallet.main_address().await?;
            (seed, address)
        };

        let restore_dir = tempfile::TempDir::new().context("creating restore dir")?;
        let restore_path = restore_dir.path().join("restored_wallet").display().to_string();

        let restored = WalletHandle::open_or_create_from_seed(
            restore_path,
            seed,
            Network::Mainnet,
            0,
            false,
            ctx.daemon.clone(),
        )
        .await
        .context("restoring wallet from seed")?;

        restored.unsafe_prepare_for_regtest().await;

        assert_eq!(
            restored.main_address().await?,
            original_address,
            "restored address differs from original"
        );

        Ok(())
    })
    .await
}

#[tokio::test]
#[serial]
async fn test_records_wallet_in_recent_wallets() -> Result<()> {
    setup_test(|ctx| async move {
        let db_dir = tempfile::TempDir::new().context("creating db dir")?;
        let db = Arc::new(monero_sys::Database::new(db_dir.path().to_path_buf()).await?);

        let wallets = Wallets::new(
            ctx.wallet_dir.path().to_path_buf(),
            WALLET_NAME.to_string(),
            ctx.daemon.clone(),
            Network::Mainnet,
            true,
            None,
            Some(db.clone()),
        )
        .await
        .context("creating wallets")?;

        {
            let main_wallet = wallets.main_wallet().await;
            main_wallet.refresh_blocking().await?;
            let _ = main_wallet.main_address().await?;
        }

        let recent = wallets.get_recent_wallets().await?;
        assert!(!recent.is_empty());
        assert!(
            recent.iter().any(|p| p.contains(WALLET_NAME)),
            "expected wallet name '{WALLET_NAME}' not found in recent list"
        );

        ctx.shutdown_test_wallets(wallets).await?;

        Ok(())
    })
    .await
}

#[tokio::test]
#[serial]
async fn test_change_monero_node_to_same_daemon() -> Result<()> {
    setup_test(|ctx| async move {
        let wallet = ctx.open_regtest_wallet(ctx.wallet_path()).await?;

        wallet.refresh_blocking().await?;
        let height_before = wallet.blockchain_height().await?;

        let daemon = ctx.daemon.clone();
        wallet
            .call(move |w| w.set_daemon(&daemon))
            .await??;

        wallet.refresh_blocking().await?;
        let height_after = wallet.blockchain_height().await?;

        assert!(height_after >= height_before);
        Ok(())
    })
    .await
}

#[tokio::test]
#[serial]
async fn test_change_monero_node_to_different_daemon_and_resyncs() -> Result<()> {
    setup_test(|ctx| async move {
        let wallet = ctx.open_regtest_wallet(ctx.wallet_path()).await?;

        ctx.generate_blocks(12).await?;
        wallet.refresh_blocking().await?;
        let height_a = wallet.blockchain_height().await?;

        let cli_b = harness::docker_client();
        let (_monero_b, monerod_b, _wallet_b) =
            Monero::new(cli_b, vec!["secondary_wallet"]).await?;

        let port_b = monerod_b
            .ports()
            .map_to_host_port_ipv4(image::RPC_PORT)
            .ok_or_else(|| anyhow::anyhow!("failed to map secondary monerod RPC port"))?;

        let daemon_b = Daemon {
            hostname: "127.0.0.1".to_string(),
            port: port_b,
            ssl: false,
        };

        let daemon = daemon_b.clone();
        wallet
            .call(move |w| w.set_daemon(&daemon))
            .await??;

        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                wallet.refresh_blocking().await?;
                let h = wallet.blockchain_height().await?;
                if h < height_a {
                    return Ok::<(), anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .context("wallet did not switch to new daemon within 45 s")??;

        let height_b = wallet.blockchain_height().await?;
        assert!(height_b < height_a);

        Ok(())
    })
    .await
}
