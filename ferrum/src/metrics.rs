use ferrum_sdk::Score;
use lazy_static::lazy_static;
use prometheus::{
    Encoder, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder, histogram_opts,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

lazy_static! {
    static ref REGISTRY: Registry = Registry::new();
    static ref REQUESTS_TOTAL: IntCounterVec = {
        let opts = Opts::new("waf_requests_total", "Total WAF decisions by action");
        let counter = IntCounterVec::new(opts, &["action"]).unwrap();
        REGISTRY.register(Box::new(counter.clone())).unwrap();
        counter
    };
    static ref RULE_SCORE: HistogramVec = {
        let opts = histogram_opts!(
            "waf_rule_score",
            "Distribution of rule scores",
            vec![
                0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0
            ]
        );
        let hist = HistogramVec::new(opts, &["rule_id"]).unwrap();
        REGISTRY.register(Box::new(hist.clone())).unwrap();
        hist
    };
    static ref BODY_BUFFER_SIZE: prometheus::Histogram = {
        let opts = histogram_opts!(
            "waf_body_buffer_size_bytes",
            "Size of buffered request bodies",
            vec![1024.0, 4096.0, 16384.0, 65536.0, 131072.0, 262144.0]
        );
        let hist = prometheus::Histogram::with_opts(opts).unwrap();
        REGISTRY.register(Box::new(hist.clone())).unwrap();
        hist
    };
}

/// Records a WAF decision in `waf_requests_total` and `waf_rule_score`.
pub fn record_decision(action: &str, rule_id: &str, score: Score) {
    REQUESTS_TOTAL.with_label_values(&[action]).inc();
    RULE_SCORE
        .with_label_values(&[rule_id])
        .observe(f64::from(score));
}

/// Records the size of a buffered request body.
pub fn record_body_size(bytes: usize) {
    BODY_BUFFER_SIZE.observe(bytes as f64);
}

/// Spawns a background tokio task serving `/metrics` on `addr`.
pub fn spawn_metrics_server_on(addr: &str) {
    // Force lazy_static initialisation so counters exist before first scrape.
    lazy_static::initialize(&REQUESTS_TOTAL);
    lazy_static::initialize(&RULE_SCORE);
    lazy_static::initialize(&BODY_BUFFER_SIZE);

    let addr = addr.to_owned();
    tokio::spawn(async move {
        let listener = TcpListener::bind(&addr)
            .await
            .unwrap_or_else(|e| panic!("metrics: bind to '{addr}' failed: {e}"));
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                let mut body = Vec::new();
                if TextEncoder::new()
                    .encode(&REGISTRY.gather(), &mut body)
                    .is_err()
                {
                    return;
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if stream.write_all(response.as_bytes()).await.is_err() {
                    return;
                }
                let _ = stream.write_all(&body).await;
            });
        }
    });
}
