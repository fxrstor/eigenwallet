mod harness;

use anyhow::Result;
use harness::setup_test;
use monero_address::Network;
use monero_sys::WalletHandle;
use serial_test::serial;
use swap_core::monero::Amount;
use tempfile::TempDir;

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

        context.generate_blocks(12).await?; 
        context.sync_wallet(&main_wallet).await?;

        let unlocked = main_wallet.unlocked_balance().await?;
        assert_eq!(unlocked.as_pico(), amount);
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

        context.generate_blocks(12).await?;
        context.sync_wallet(&main_wallet).await?;

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

        context.generate_blocks(12).await?;
        context.sync_wallet(&alice_wallet).await?;

        let bob_dir = TempDir::new()?;
        let bob_wallet = WalletHandle::open_or_create(
            bob_dir.path().join("bob_wallet").display().to_string(),
            context.daemon.clone(),
            Network::Mainnet,
            true,
        ).await?;
        
        bob_wallet.unsafe_prepare_for_regtest().await;

        let bob_address = bob_wallet.main_address().await?;

        let send_amount = 100_000_000_000u64; // 0.1 XMR
        alice_wallet.transfer_single_destination(&bob_address, Amount::from_pico(send_amount)).await?;

        context.generate_blocks(12).await?;
        context.sync_wallet(&bob_wallet).await?;
        context.wait_for_unlocked_balance(&bob_wallet, send_amount, 120).await?;
        let bob_unlocked = bob_wallet.unlocked_balance().await?;
        assert_eq!(bob_unlocked.as_pico(), send_amount);
        Ok(())
    }).await?;
    Ok(())
}
