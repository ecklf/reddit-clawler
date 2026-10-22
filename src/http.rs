use rand::Rng;
use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime},
};
use wreq::{
    cookie::Jar, header::RETRY_AFTER, Client, Method, RequestBuilder, Response, StatusCode,
};
use wreq_util::Emulation;

const MAX_RETRIES: u32 = 3;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(120);

/// Keep the TLS handshake, HTTP/2 settings, header order and browser headers consistent.
pub fn build_client(cookie_jar: Arc<Jar>) -> wreq::Result<Client> {
    Client::builder()
        .emulation(Emulation::SafariIos18_1_1)
        .cookie_provider(cookie_jar)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(300))
        .pool_idle_timeout(Duration::from_secs(60))
        .pool_max_idle_per_host(10)
        // Retries are handled below so every attempt uses the same bounded backoff.
        .retry(wreq::retry::Policy::never())
        .build()
}

pub trait RequestBuilderExt {
    /// Retry replayable GET/HEAD requests after transient failures.
    fn send_with_retry(self) -> impl Future<Output = wreq::Result<Response>> + Send;
}

impl RequestBuilderExt for RequestBuilder {
    async fn send_with_retry(self) -> wreq::Result<Response> {
        let (client, request) = self.build_split();
        let request = request?;
        if !matches!(*request.method(), Method::GET | Method::HEAD) {
            return client.execute(request).await;
        }
        let mut pending = request.try_clone();
        let mut result = client.execute(request).await;

        for attempt in 0..MAX_RETRIES {
            let delay = match &result {
                Ok(response) if retryable_status(response.status()) => {
                    let server_delay = response
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| retry_after(value, SystemTime::now()));
                    let delay = server_delay.unwrap_or_default().max(backoff(attempt));
                    // Return the response rather than retrying earlier than the server allows.
                    if delay > MAX_RETRY_DELAY {
                        break;
                    }
                    delay
                }
                Err(error)
                    if !error.is_tls()
                        && (error.is_timeout()
                            || error.is_connect()
                            || error.is_connection_reset()) =>
                {
                    backoff(attempt)
                }
                _ => break,
            };

            let Some(request) = pending.take() else {
                break;
            };
            pending = request.try_clone();
            // Release the failed response before sleeping, including its connection/body.
            drop(result);
            tokio::time::sleep(delay).await;
            result = client.execute(request).await;
        }

        result
    }
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn backoff(attempt: u32) -> Duration {
    let base_ms = 500 * 2_u64.pow(attempt);
    Duration::from_millis(base_ms + rand::thread_rng().gen_range(0..=base_ms / 2))
}

fn retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(now).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_retry_after_seconds_and_dates() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(retry_after(" 12 ", now), Some(Duration::from_secs(12)));
        let date = httpdate::fmt_http_date(now + Duration::from_secs(30));
        assert_eq!(retry_after(&date, now), Some(Duration::from_secs(30)));
        let past = httpdate::fmt_http_date(now - Duration::from_secs(30));
        assert_eq!(retry_after(&past, now), Some(Duration::ZERO));
        assert_eq!(retry_after("invalid", now), None);
    }
}
