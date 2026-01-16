use anyhow::{Context, Result};
use monero_harness::{image, Monero};
use monero_sys::Daemon;
use monero_wallet::Wallets;
use std::sync::OnceLock;
use tempfile::TempDir;
use testcontainers::clients::Cli;

use std::future::Future;
use tokio::task::JoinHandle;

pub const WALLET_NAME: &str = "test_wallet";

fn init_globals() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let subscriber = tracing_subscriber::fmt().with_env_filter("info,monero_wallet=debug,monero_sys=debug").with_test_writer();
        let _ = subscriber.try_init();
    });
}

#[derive(Clone)]
pub struct TestContext {
    pub monero: Monero,
    pub wallet_dir: TempDir,
    pub daemon: Daemon,
    pub wallet_name: String,
}

impl TestContext {
    pub async fn create_wallets(&self) -> Result<Wallets> {
        Wallets::new(
            self.wallet_dir.path().to_path_buf(),
            self.wallet_name.clone(),
            self.daemon.clone(),
            monero::Network::Regtest,
            true,
            None,
            None,
        ).await.context("creating wallets")
    }
}

pub async fn setup_test<F, Fut>(test: F) -> Result<()>
where
    F: FnOnce(TestContext) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    init_globals();

    let cli = Cli::default();
    let wallet_name = WALLET_NAME.to_string();

    let (monero, monerod_container, wallet_container) = Monero::new(&cli, vec![wallet_name.clone()]).await.context("spawning monero containers")?;
    let monerod_port = monerod_container.ports().map_to_host_port_ipv4(image::RPC_PORT).context("rpc port should be mapped to some external port")?;
    let daemon = Daemon {
        hostname: "127.0.0.1".to_string(),
        port: monerod_port,
        ssl: false,
    };
    let wallet_dir = TempDir::new().context("creating temp wallet dir")?;
    let ctx = TestContext {
        monero: monero.clone(),
        wallet_dir,
        daemon: daemon.clone(),
        wallet_name: wallet_name.clone(),
    };
    monero.init_miner().await.context("initializing miner")?;
    monero.start_miner().await.context("starting miner")?;
    
    let test_handle: JoinHandle<Result<()>> = tokio::spawn(async move {
        test(ctx).await
    });

    let _ = monero.stop_miner().await;
    drop(monerod_container);
    drop(wallet_container);

    let test_result = match test_handle.await {
        Ok(inner_res) => inner_res.context("test future returned an error")?,
        Err(join_err) => {
            return Err(anyhow::anyhow!("test task panicked or was cancelled: {:?}", join_err));
        }
    };

    Ok(test_result)
}
