mod harness;

use anyhow::Result;
use harness::setup_test;
use monero_address::Network;
use monero_wallet::Wallets;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_create_wallet() -> Result<()> {
    setup_test(|context| async move {
        let wallets = context.create_wallets().await?;
        let main_wallet = wallets.main_wallet().await;
        let address = main_wallet.main_address().await?;
        assert_eq!(address.network(), Network::Mainnet);
        assert!(address.to_string().starts_with('4'));
        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_open_existing_wallet() -> Result<()> {
    setup_test(|context| async move {
        let _wallets = context.create_wallets().await?;
        let initial_address = _wallets.main_wallet().await.main_address().await?;
        drop(_wallets);

        let wallets = Wallets::new(
            context.wallet_dir.path().to_path_buf(),
            harness::WALLET_NAME.to_string(),
            context.daemon.clone(),
            Network::Mainnet,
            true,
            None,
            None,
        ).await?;

        let main_wallet = wallets.main_wallet().await;
        let address = main_wallet.main_address().await?;

        assert_eq!(address.network(), Network::Mainnet);
        assert_eq!(address.to_string(), initial_address.to_string());

        Ok(())
    }).await?;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_restore_wallet_from_seed() -> Result<()> {
    setup_test(|context| async move {
        let wallets = context.create_wallets().await?;
        let main_wallet = wallets.main_wallet().await;
        let seed = main_wallet.seed().await?;
        let address = main_wallet.main_address().await?;

        let restore_dir = tempfile::TempDir::new()?;
        let restore_name = "restored_wallet";

        use monero_sys::WalletHandle;

        let restored_wallet = WalletHandle::open_or_create_from_seed(
            restore_dir.path().join(restore_name).display().to_string(),
            seed.clone(),
            Network::Mainnet,
            0,
            true,
            context.daemon.clone(),
        ).await?;

        restored_wallet.unsafe_prepare_for_regtest().await;

        let restored_address = restored_wallet.main_address().await?;
        assert_eq!(address, restored_address);

        Ok(())
    }).await?;
    Ok(())
}
