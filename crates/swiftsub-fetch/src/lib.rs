use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::{Proxy, StatusCode, redirect::Policy};

const MAX_CONTENT_BYTES: usize = 10 * 1024 * 1024;
const DEFAULT_USER_AGENT: &str = "clash.meta/1.19.0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyMode {
    Direct,
    System,
    Custom(String),
}

#[derive(Clone, Debug)]
pub struct FetchOptions {
    pub user_agent: String,
    pub timeout: Duration,
    pub proxy: ProxyMode,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.to_string(),
            timeout: Duration::from_secs(20),
            proxy: ProxyMode::System,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CacheValidators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubscriptionInfo {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    pub expire: Option<i64>,
}

impl SubscriptionInfo {
    pub fn used(&self) -> u64 {
        self.upload.saturating_add(self.download)
    }

    pub fn remaining(&self) -> u64 {
        self.total.saturating_sub(self.used())
    }

    pub fn to_header_value(&self) -> String {
        let mut value = format!(
            "upload={}; download={}; total={}",
            self.upload, self.download, self.total
        );
        if let Some(expire) = self.expire {
            value.push_str(&format!("; expire={expire}"));
        }
        value
    }
}

#[derive(Clone, Debug)]
pub struct FetchResult {
    pub content: String,
    pub final_url: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub info: Option<SubscriptionInfo>,
    pub update_interval_hours: Option<u32>,
}

#[derive(Clone, Debug)]
pub enum FetchOutcome {
    Fetched(FetchResult),
    NotModified {
        info: Option<SubscriptionInfo>,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("HTTP {0}: {1}")]
    HttpStatus(u16, String),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("subscription response is too large (limit {} MB)", MAX_CONTENT_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("subscription response is empty")]
    Empty,
}

pub fn fetch_subscription(
    url: &str,
    options: &FetchOptions,
    cache: &CacheValidators,
) -> Result<FetchOutcome, FetchError> {
    let client = build_client(options)?;

    let mut request = client.get(url).header(ACCEPT, "*/*");
    if let Some(etag) = &cache.etag
        && !etag.is_empty()
    {
        request = request.header(IF_NONE_MATCH, etag);
    }
    if let Some(last_modified) = &cache.last_modified
        && !last_modified.is_empty()
    {
        request = request.header(IF_MODIFIED_SINCE, last_modified);
    }

    let response = request.send()?;
    let status = response.status();
    let headers = response.headers().clone();
    let final_url = response.url().to_string();

    let info = headers
        .get("subscription-userinfo")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_subscription_info);
    let etag = headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let last_modified = headers
        .get(LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let update_interval_hours = headers
        .get("profile-update-interval")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|hours| *hours > 0);

    if status == StatusCode::NOT_MODIFIED {
        return Ok(FetchOutcome::NotModified {
            info,
            etag,
            last_modified,
        });
    }

    if !status.is_success() {
        let message = response.text().unwrap_or_default();
        return Err(FetchError::HttpStatus(
            status.as_u16(),
            message.chars().take(200).collect(),
        ));
    }

    if let Some(length) = headers
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        && length > MAX_CONTENT_BYTES
    {
        return Err(FetchError::TooLarge);
    }

    let content = response.text()?;
    if content.trim().is_empty() {
        return Err(FetchError::Empty);
    }
    if content.len() > MAX_CONTENT_BYTES {
        return Err(FetchError::TooLarge);
    }

    Ok(FetchOutcome::Fetched(FetchResult {
        content,
        final_url,
        etag,
        last_modified,
        info,
        update_interval_hours,
    }))
}

fn build_client(options: &FetchOptions) -> Result<Client, FetchError> {
    let timeout = options.timeout.max(Duration::from_secs(3));
    let mut builder = Client::builder()
        .user_agent(options.user_agent.clone())
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(10))
        .redirect(Policy::limited(10));

    builder = match &options.proxy {
        ProxyMode::Direct => builder.no_proxy(),
        ProxyMode::System => builder,
        ProxyMode::Custom(url) => {
            let proxy = Proxy::all(url).map_err(|error| {
                FetchError::HttpStatus(0, format!("invalid proxy '{url}': {error}"))
            })?;
            builder.proxy(proxy)
        }
    };

    builder.build().map_err(FetchError::Network)
}

pub fn parse_subscription_info(raw: &str) -> Option<SubscriptionInfo> {
    let mut info = SubscriptionInfo::default();
    let mut found = false;

    for part in raw.split(';') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim();
        match key.as_str() {
            "upload" => {
                if let Ok(value) = value.parse() {
                    info.upload = value;
                    found = true;
                }
            }
            "download" => {
                if let Ok(value) = value.parse() {
                    info.download = value;
                    found = true;
                }
            }
            "total" => {
                if let Ok(value) = value.parse() {
                    info.total = value;
                    found = true;
                }
            }
            "expire" => {
                if let Ok(value) = value.parse::<i64>() {
                    info.expire = (value > 0).then_some(value);
                    found = true;
                }
            }
            _ => {}
        }
    }

    found.then_some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subscription_userinfo() {
        let info =
            parse_subscription_info("upload=1024; download=2048; total=1048576; expire=1700000000")
                .unwrap();
        assert_eq!(info.upload, 1024);
        assert_eq!(info.download, 2048);
        assert_eq!(info.total, 1_048_576);
        assert_eq!(info.expire, Some(1_700_000_000));
        assert_eq!(info.used(), 3072);
        assert_eq!(info.remaining(), 1_045_504);
    }

    #[test]
    fn formats_response_header() {
        let info = SubscriptionInfo {
            upload: 1,
            download: 2,
            total: 3,
            expire: Some(4),
        };
        assert_eq!(
            info.to_header_value(),
            "upload=1; download=2; total=3; expire=4"
        );
    }

    #[test]
    fn ignores_invalid_userinfo() {
        assert!(parse_subscription_info("foo=bar").is_none());
        assert!(parse_subscription_info("").is_none());
    }
}

#[cfg(test)]
mod http_tests {
    use std::thread;
    use std::time::Duration;

    use tiny_http::{Header, Response, Server, StatusCode};

    use super::*;

    fn start_server(
        status: u16,
        body: &'static str,
        extra_headers: Vec<(&'static str, &'static str)>,
    ) -> (String, std::net::SocketAddr, thread::JoinHandle<()>) {
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        let url = format!("http://{addr}/subscription");
        let handle = thread::spawn(move || {
            if let Ok(Some(request)) = server.recv_timeout(Duration::from_secs(5)) {
                let mut response = Response::from_string(body).with_status_code(StatusCode(status));
                for (name, value) in extra_headers {
                    response
                        .add_header(Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap());
                }
                let _ = request.respond(response);
            }
        });
        (url, addr, handle)
    }

    #[test]
    fn fetches_subscription_and_parses_headers() {
        let (url, _addr, handle) = start_server(
            200,
            "proxies: []\n",
            vec![
                (
                    "subscription-userinfo",
                    "upload=100; download=200; total=1000; expire=1700000000",
                ),
                ("profile-update-interval", "12"),
                ("etag", "\"etag-1\""),
            ],
        );

        let outcome = fetch_subscription(
            &url,
            &FetchOptions {
                proxy: ProxyMode::Direct,
                ..Default::default()
            },
            &CacheValidators::default(),
        )
        .unwrap();

        let FetchOutcome::Fetched(result) = outcome else {
            panic!("expected fetched response");
        };
        assert_eq!(result.content, "proxies: []\n");
        assert_eq!(result.etag.as_deref(), Some("\"etag-1\""));
        assert_eq!(result.update_interval_hours, Some(12));
        let info = result.info.unwrap();
        assert_eq!(info.used(), 300);
        assert_eq!(info.remaining(), 700);
        assert_eq!(info.expire, Some(1_700_000_000));

        handle.join().unwrap();
    }

    #[test]
    fn reports_not_modified() {
        let (url, _addr, handle) = start_server(304, "", vec![("etag", "\"same\"")]);

        let outcome = fetch_subscription(
            &url,
            &FetchOptions {
                proxy: ProxyMode::Direct,
                ..Default::default()
            },
            &CacheValidators {
                etag: Some("\"same\"".into()),
                last_modified: None,
            },
        )
        .unwrap();

        assert!(matches!(outcome, FetchOutcome::NotModified { .. }));
        handle.join().unwrap();
    }

    #[test]
    fn reports_http_errors() {
        let (url, _addr, handle) = start_server(500, "boom", vec![]);
        let error = fetch_subscription(
            &url,
            &FetchOptions {
                proxy: ProxyMode::Direct,
                ..Default::default()
            },
            &CacheValidators::default(),
        )
        .unwrap_err();

        assert!(matches!(error, FetchError::HttpStatus(500, _)));
        handle.join().unwrap();
    }
}
