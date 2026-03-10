use anyhow::{Context, Result};
use monero_harness::{image, Monero};
use monero_sys::Daemon;
use monero_address::Network;
use monero_wallet::Wallets;
use std::sync::OnceLock;
use tempfile::TempDir;
use monero_harness::Cli;

static INIT_RUSTLS: std::sync::Once = std::sync::Once::new();
static INIT_TRACING: OnceLock<()> = OnceLock::new();

pub const WALLET_NAME: &str = "test_wallet";

fn init_rustls() {
    INIT_RUSTLS.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install rustls ring crypto provider");
    });
}

fn init_globals() {
    init_rustls();
    INIT_TRACING.get_or_init(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter("info,monero_wallet=debug,monero_sys=debug")
            .with_test_writer();
        let _ = subscriber.try_init();
    });
}

#[allow(dead_code)]
pub struct TestContext {
    pub monero: Monero,
    pub wallet_dir: TempDir,
    pub daemon: Daemon,
    pub wallet_name: String,
}

#[allow(dead_code)]
impl TestContext {
    pub async fn create_wallets(&self) -> Result<Wallets> {
        Wallets::new(
            self.wallet_dir.path().to_path_buf(),
            self.wallet_name.clone(),
            self.daemon.clone(),
            Network::Mainnet,
            true,
            None,
            None,
        )
        .await
        .context("creating wallets")
    }

    pub async fn generate_blocks(&self, n: usize) -> Result<()> {
        for _ in 0..n {
            self.monero.generate_block().await.context("generating block")?;
        }
        
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        Ok(())
    }
    
    pub async fn sync_wallet(&self, wallet: &monero_sys::WalletHandle) -> Result<()> {
        use std::time::Duration;
        const SYNC_TIMEOUT_SECS: u64 = 180;
        tokio::time::timeout(Duration::from_secs(SYNC_TIMEOUT_SECS), async {
            wallet.wait_until_synced(monero_sys::no_listener()).await
        })
        .await
        .context("wallet sync timeout")?
        .context("wallet wait_until_synced error")?;
        Ok(())
    }

    pub async fn wait_for_unlocked_balance(&self, wallet: &monero_sys::WalletHandle, expected_pico: u64, timeout_secs: u64) -> Result<()> {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        while start.elapsed().as_secs() < timeout_secs {
            let unlocked = wallet.unlocked_balance().await.context("reading unlocked balance")?;
            if unlocked.as_pico() >= expected_pico {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        anyhow::bail!("timeout waiting for unlocked balance >= {}", expected_pico);
    }

}

pub async fn setup_test<F, Fut>(test: F) -> Result<()>
where
    F: FnOnce(TestContext) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<()>> + Send + 'static,
{
    init_globals();
    let cli = Cli::default();
    let wallet_name_string = WALLET_NAME.to_string();
    let (monero, monerod_container, _wallet_container) = Monero::new(&cli, vec![WALLET_NAME, "miner"]).await.context("spawning monero containers")?;

    let monerod_port = monerod_container
        .ports()
        .map_to_host_port_ipv4(image::RPC_PORT)
        .ok_or_else(|| anyhow::anyhow!("rpc port should be mapped to some external port"))
        .context("rpc port should be mapped to some external port")?;

    let daemon = Daemon {
        hostname: "127.0.0.1".to_string(),
        port: monerod_port,
        ssl: false,
    };

    monero.init_miner().await.context("initializing miner")?;
    monero.start_miner().await.context("starting miner")?;

    let wallet_dir = TempDir::new().context("creating temp wallet dir")?;
    let ctx = TestContext {
        monero,
        wallet_dir,
        daemon: daemon.clone(),
        wallet_name: wallet_name_string,
    };
    
    test(ctx).await
}
