mod harness;

use anyhow::Result;
use harness::setup_test;
use monero_address::Network;
use monero_sys::WalletHandle;
use serial_test::serial;
use std::time::Duration;
use tokio::time::timeout;
use swap_core::monero::Amount;

async fn wait_for_condition<F, Fut>(mut f: F, max_wait: Duration, poll_interval: Duration) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    timeout(max_wait, async {
        loop {
            if f().await? {
                return Ok(());
            }
            tokio::time::sleep(poll_interval).await;
        }
    }).await.map_err(|_| anyhow::anyhow!("timed out waiting for condition"))?
}

#[tokio::test]
#[serial]
async fn test_receive_funds() -> Result<()> {
    setup_test(|context| async move {
        let wallets = context.create_wallets().await?;
        let main_wallet = wallets.main_wallet().await;
        let address = main_wallet.main_address().await?;

        let miner_wallet = context.monero.wallet("miner")?;
        let amount = 1_000_000_000_000u64; // 1 XMR
        miner_wallet.transfer(&address, amount).await?;

        for _ in 0..60 {
            context.monero.generate_block().await?;
        }
        main_wallet.wait_until_synced(monero_sys::no_listener()).await?;
        wait_for_condition(
            || {
                let main_wallet = main_wallet.clone();
                async move {
                    main_wallet.wait_until_synced(monero_sys::no_listener()).await?;
                    let b = main_wallet.unlocked_balance().await?;
                    Ok(b.as_pico() > 0)
                }
            },
            Duration::from_secs(20),
            Duration::from_millis(500),
        ).await?;

        let wallet_balance = main_wallet.unlocked_balance().await?;
        assert_eq!(wallet_balance.as_pico(), amount);
        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_transaction_history() -> Result<()> {
    setup_test(|context| async move {
        let wallets = context.create_wallets().await?;
        let main_wallet = wallets.main_wallet().await;
        let address = main_wallet.main_address().await?;

        let miner_wallet = context.monero.wallet("miner")?;
        let amount = 1_000_000_000_000u64; // 1 XMR
        miner_wallet.transfer(&address, amount).await?;

        for _ in 0..70 {
            context.monero.generate_block().await?;
        }
        main_wallet.wait_until_synced(monero_sys::no_listener()).await?;

        wait_for_condition(
            || {
                let main_wallet = main_wallet.clone();
                async move {
                    main_wallet.wait_until_synced(monero_sys::no_listener()).await?;
                    let h = main_wallet.history().await?;
                    Ok(!h.is_empty())
                }
            },
            Duration::from_secs(20),
            Duration::from_millis(500),
        ).await?;

        let transactions = main_wallet.history().await?;
        assert!(!transactions.is_empty());
        let tx = transactions.iter().find(|t| t.amount.as_pico() == amount && t.direction == monero_sys::TransactionDirection::In).expect("expected incoming tx not found");
        assert_eq!(tx.direction, monero_sys::TransactionDirection::In);
        assert_eq!(tx.amount.as_pico(), amount);
        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_transfer_funds() -> Result<()> {
    setup_test(|context| async move {
        let wallets_alice = context.create_wallets().await?;
        let alice_wallet = wallets_alice.main_wallet().await;
        let alice_address = alice_wallet.main_address().await?;

        let miner_wallet = context.monero.wallet("miner")?;
        let amount = 1_000_000_000_000u64; // 1 XMR
        miner_wallet.transfer(&alice_address, amount).await?;

        for _ in 0..70 {
            context.monero.generate_block().await?;
        }
        alice_wallet.wait_until_synced(monero_sys::no_listener()).await?;
        wait_for_condition(
            || {
                let alice_wallet = alice_wallet.clone();
                async move {
                    alice_wallet.wait_until_synced(monero_sys::no_listener()).await?;
                    let b = alice_wallet.unlocked_balance().await?;
                    Ok(b.as_pico() >= amount)
                }
            },
            Duration::from_secs(40),
            Duration::from_millis(500),
        ).await?;

        let bob_dir = tempfile::TempDir::new()?;
        let bob_wallet = WalletHandle::open_or_create(
            bob_dir.path().join("bob_wallet").display().to_string(),
            context.daemon.clone(),
            Network::Mainnet,
            true,
        ).await?;

        let bob_address = bob_wallet.main_address().await?;

        let send_amount = 100_000_000_000u64; // 0.1 XMR
        alice_wallet.transfer_single_destination(&bob_address, Amount::from_pico(send_amount)).await?;

        for _ in 0..70 {
            context.monero.generate_block().await?;
        }
        
        bob_wallet.wait_until_synced(monero_sys::no_listener()).await?;
        wait_for_condition(
            || {
                let bob_wallet = bob_wallet.clone();
                async move {
                    bob_wallet.wait_until_synced(monero_sys::no_listener()).await?;
                    let b = bob_wallet.unlocked_balance().await?;
                    Ok(b.as_pico() >= send_amount)
                }
            },
            Duration::from_secs(60),
            Duration::from_millis(500),
        ).await?;

        let bob_unlocked = bob_wallet.unlocked_balance().await?;
        assert_eq!(bob_unlocked.as_pico(), send_amount);
        Ok(())
    }).await?;
    Ok(())
}
