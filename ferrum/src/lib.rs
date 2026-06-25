mod loader;
mod metrics;
mod proxy;

use std::sync::Arc;

use pingora::server::Server;
use pingora::server::configuration::Opt;
use pingora_proxy::http_proxy_service;

use crate::loader::load_config;
use crate::proxy::FerrumProxy;

/// Initialises all plugins collected via [`ferrum_sdk::inventory`], loads `config_path`, and
/// starts the Pingora proxy.  Blocks until the server is shut down.
pub fn run(config_path: &str) {
    ferrum_sdk::init_from_inventory();

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let opt = Opt::default();
    let mut server = Server::new(Some(opt)).unwrap();
    server.bootstrap();

    let ferrum_config = Arc::new(load_config(config_path));

    let listen_addr = ferrum_config.server.listen.clone();
    let metrics_addr = ferrum_config.server.metrics_listen.clone();

    let proxy = FerrumProxy::new(Arc::clone(&ferrum_config));
    let mut proxy_service = http_proxy_service(&server.configuration, proxy);
    proxy_service.add_tcp(&listen_addr);
    server.add_service(proxy_service);

    let _metrics_handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            metrics::spawn_metrics_server_on(&metrics_addr);
            std::future::pending::<()>().await;
        });
    });

    server.run_forever();
}
