use std::time::Duration;

use crate::config::HttpSection;

/// Build a shared `reqwest::Client` configured from the `[http]` config section.
pub fn build_http_client(config: &HttpSection) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
        .timeout(Duration::from_secs(config.request_timeout_secs))
        .build()
        .expect("valid reqwest client config")
}

/// Build an HTTP client that exposes redirects to webhook delivery as 3xx responses.
pub fn build_webhook_http_client(config: &HttpSection) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
        .timeout(Duration::from_secs(config.request_timeout_secs))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("valid reqwest client config")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_http_client_uses_configured_timeouts() {
        let config = HttpSection::default();
        let _client = build_http_client(&config);
        let _webhook_client = build_webhook_http_client(&config);
    }
}
