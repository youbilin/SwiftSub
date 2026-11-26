use std::io::Read;
use std::sync::{Mutex, OnceLock};
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
        update_interval_hours: Option<u32>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("response read error: {0}")]
    Read(#[from] std::io::Error),
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
            update_interval_hours,
        });
    }

    if !status.is_success() {
        let mut bytes = Vec::new();
        let _ = response.take(1024).read_to_end(&mut bytes);
        let message = String::from_utf8_lossy(&bytes);
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

    let charset = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value.split(';').find_map(|part| {
                let (key, value) = part.trim().split_once('=')?;
                key.eq_ignore_ascii_case("charset")
                    .then(|| value.trim().trim_matches('"'))
            })
        });
    let bytes = read_bounded(response, MAX_CONTENT_BYTES)?;
    let encoding = charset
        .and_then(|value| encoding_rs::Encoding::for_label(value.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    let (content, _, _) = encoding.decode(&bytes);
    let content = content.into_owned();
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
    static CLIENTS: OnceLock<Mutex<Vec<(FetchOptions, Client)>>> = OnceLock::new();
    let cacheable = !matches!(options.proxy, ProxyMode::System);
    let clients = CLIENTS.get_or_init(|| Mutex::new(Vec::new()));
    if cacheable
        && let Ok(cache) = clients.lock()
        && let Some((_, client)) = cache.iter().find(|(key, _)| key == options)
    {
        return Ok(client.clone());
    }
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

    let client = builder.build().map_err(FetchError::Network)?;
    if cacheable && let Ok(mut cache) = clients.lock() {
        if let Some((_, existing)) = cache.iter().find(|(key, _)| key == options) {
            return Ok(existing.clone());
        }
        if cache.len() >= 8 {
            cache.remove(0);
        }
        cache.push((options.clone(), client.clone()));
    }
    Ok(client)
}

/// Fetch binary rule sets without passing their bytes through charset decoding.
pub fn fetch_rule_bytes(url: &str, options: &FetchOptions) -> Result<Vec<u8>, FetchError> {
    let response = build_client(options)?.get(url).send()?;
    if !response.status().is_success() {
        return Err(FetchError::HttpStatus(
            response.status().as_u16(),
            "规则集读取失败".into(),
        ));
    }
    if let Some(length) = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        && length > MAX_CONTENT_BYTES
    {
        return Err(FetchError::TooLarge);
    }
    let bytes = read_bounded(response, MAX_CONTENT_BYTES)?;
    if bytes.is_empty() {
        return Err(FetchError::Empty);
    }
    Ok(bytes)
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, FetchError> {
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(FetchError::TooLarge);
    }
    Ok(bytes)
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: String,
    pub title: String,
    pub notes: String,
    pub html_url: String,
    pub download_url: Option<String>,
    pub published_at: Option<String>,
    pub has_update: bool,
}

#[derive(serde::Deserialize)]
struct GithubReleaseResponse {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GithubReleaseAsset>,
}

#[derive(serde::Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
}

fn parse_semver(raw: &str) -> Option<(u64, u64, u64, bool)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let without_build = trimmed.split('+').next().unwrap_or_default();
    let (core, is_stable) = match without_build.split_once('-') {
        Some((core, _)) => (core, false),
        None => (without_build, true),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch, is_stable))
}

pub fn is_newer_version(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => {
            let l = latest.trim().trim_start_matches(['v', 'V']);
            let c = current.trim().trim_start_matches(['v', 'V']);
            !l.is_empty() && l != c
        }
    }
}

fn parse_github_release_json(body: &str, current_version: &str) -> Result<ReleaseInfo, String> {
    let parsed: GithubReleaseResponse =
        serde_json::from_str(body).map_err(|error| format!("更新接口返回格式无效：{error}"))?;
    let version = parsed
        .tag_name
        .trim()
        .trim_start_matches(['v', 'V'])
        .to_string();
    let title = parsed
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| parsed.tag_name.clone());
    let download_url = parsed
        .assets
        .iter()
        .find(|asset| {
            let lower = asset.name.to_ascii_lowercase();
            lower.ends_with(".exe") || lower.ends_with(".msi")
        })
        .or_else(|| {
            parsed
                .assets
                .iter()
                .find(|asset| asset.name.to_ascii_lowercase().ends_with(".zip"))
        })
        .map(|asset| asset.browser_download_url.clone());
    let has_update = is_newer_version(&version, current_version);
    Ok(ReleaseInfo {
        version,
        title,
        notes: parsed.body.unwrap_or_default().trim().to_string(),
        html_url: parsed.html_url,
        download_url,
        published_at: parsed.published_at,
        has_update,
    })
}

/// Check the latest published release from GitHub Releases API (`owner/repo`).
pub fn check_github_release(
    repo: &str,
    current_version: &str,
    options: &FetchOptions,
) -> Result<ReleaseInfo, String> {
    let repo = repo.trim().trim_matches('/');
    if !repo.contains('/') || repo.split('/').count() != 2 {
        return Err("仓库格式应为 owner/repo".into());
    }
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut opts = options.clone();
    opts.timeout = Duration::from_secs(12);
    opts.user_agent = format!("SwiftSub/{current_version}");
    let client = build_client(&opts).map_err(|error| format!("创建网络客户端失败：{error}"))?;
    let response = client
        .get(&url)
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .map_err(|error| format!("无法连接更新服务：{error}"))?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Err(format!("仓库 {repo} 暂未发布 Release 版本"));
    }
    if !status.is_success() {
        return Err(format!(
            "更新服务返回 HTTP {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        ));
    }
    let bytes = read_bounded(response, 1024 * 1024)
        .map_err(|error| format!("读取更新信息失败：{error}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "更新信息编码无效".to_string())?;
    parse_github_release_json(&text, current_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reader_stops_before_consuming_the_response() {
        let mut input = std::io::Cursor::new(vec![b'a'; 100]);
        assert!(matches!(
            read_bounded(&mut input, 10),
            Err(FetchError::TooLarge)
        ));
        assert_eq!(input.position(), 11);
        assert_eq!(
            read_bounded(std::io::Cursor::new(b"exact"), 5).unwrap(),
            b"exact"
        );
    }

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

    #[test]
    fn parses_github_release_and_compares_versions() {
        assert!(is_newer_version("v0.2.0", "0.1.0"));
        assert!(is_newer_version("1.0.0", "0.9.9"));
        assert!(!is_newer_version("v0.1.0", "0.1.0"));
        assert!(!is_newer_version("0.0.9", "0.1.0"));

        let body = r#"{
            "tag_name": "v0.2.0",
            "name": "v0.2.0 正式版",
            "body": "支持局域网导入与更新检查",
            "html_url": "https://github.com/youbilin/SwiftSub/releases/tag/v0.2.0",
            "published_at": "2026-11-26T12:00:00Z",
            "assets": [
                {"name": "swiftsub-windows-x64.zip", "browser_download_url": "https://example.com/a.zip"},
                {"name": "swiftsub-setup.exe", "browser_download_url": "https://example.com/setup.exe"}
            ]
        }"#;
        let info = parse_github_release_json(body, "0.1.0").unwrap();
        assert_eq!(info.version, "0.2.0");
        assert!(info.has_update);
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://example.com/setup.exe")
        );
    }
}

#[cfg(test)]
mod http_tests {
    use std::thread;
    use std::time::Duration;

    use tiny_http::{Header, Response, Server, StatusCode};

    fn raw_response(response: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/subscription", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut headers = Vec::new();
            let mut byte = [0];
            while !headers.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).unwrap() == 0 {
                    return;
                }
                headers.push(byte[0]);
            }
            let _ = stream.write_all(&response);
        });
        (url, handle)
    }

    #[test]
    fn limits_chunked_responses_without_content_length() {
        let body = vec![b'a'; MAX_CONTENT_BYTES + 1];
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n",
            body.len()
        )
        .into_bytes();
        response.extend(body);
        response.extend(b"\r\n0\r\n\r\n");
        let (url, handle) = raw_response(response);
        let result = fetch_subscription(
            &url,
            &FetchOptions {
                proxy: ProxyMode::Direct,
                ..Default::default()
            },
            &CacheValidators::default(),
        );
        assert!(matches!(result, Err(FetchError::TooLarge)));
        handle.join().unwrap();
    }

    #[test]
    fn bounded_fetch_preserves_declared_charset() {
        let mut response = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=windows-1252\r\nContent-Length: 4\r\nConnection: close\r\n\r\n".to_vec();
        response.extend(b"caf\xe9");
        let (url, handle) = raw_response(response);
        let result = fetch_subscription(
            &url,
            &FetchOptions {
                proxy: ProxyMode::Direct,
                ..Default::default()
            },
            &CacheValidators::default(),
        )
        .unwrap();
        let FetchOutcome::Fetched(result) = result else {
            panic!("expected body");
        };
        assert_eq!(result.content, "café");
        handle.join().unwrap();
    }

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

    #[test]
    fn prerelease_upgrades_to_stable_same_version() {
        assert!(is_newer_version("1.1.0", "1.1.0-rc.1"));
        assert!(!is_newer_version("1.1.0-rc.1", "1.1.0"));
        assert!(is_newer_version("v1.2.0", "1.1.0"));
    }

    #[test]
    fn rejects_empty_body_and_invalid_custom_proxy() {
        let (url, _addr, handle) = start_server(200, "   \r\n  ", vec![]);
        let err = fetch_subscription(
            &url,
            &FetchOptions {
                proxy: ProxyMode::Direct,
                ..Default::default()
            },
            &CacheValidators::default(),
        )
        .unwrap_err();
        assert!(matches!(err, FetchError::Empty));
        handle.join().unwrap();

        let proxy_err = fetch_subscription(
            "http://127.0.0.1:1/sub",
            &FetchOptions {
                proxy: ProxyMode::Custom("://bad-proxy".into()),
                ..Default::default()
            },
            &CacheValidators::default(),
        )
        .unwrap_err();
        assert!(matches!(proxy_err, FetchError::HttpStatus(0, _)));
    }

    #[test]
    fn handles_malformed_github_release_json_and_prerelease_versions() {
        assert!(parse_github_release_json("not-json", "0.2.0").is_err());
        assert!(is_newer_version("v0.3.0-rc.1", "0.2.0"));
        assert!(!is_newer_version("v0.2.0", "0.2.0"));
        assert!(!is_newer_version("0.1.9", "0.2.0"));
        assert!(check_github_release("invalid-repo", "0.2.0", &FetchOptions::default()).is_err());
    }
}
