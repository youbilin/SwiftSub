use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub port: u16,
    pub token: String,
    pub bind_lan: bool,
    pub update_interval_hours: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 7899,
            token: String::new(),
            bind_lan: false,
            update_interval_hours: 24,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProfileSnapshot {
    pub yaml: String,
    pub userinfo_header: Option<String>,
    pub update_interval_hours: u32,
}

impl ProfileSnapshot {
    pub fn etag(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.yaml.as_bytes());
        let hash = hasher.finalize();
        format!("\"{}\"", hex::encode(&hash[..8]))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("invalid token")]
    InvalidToken,
    #[error("failed to start local server: {0}")]
    Start(String),
}

pub struct LocalServer {
    addr: SocketAddr,
    token: String,
    server: Arc<Server>,
    shutdown: Arc<AtomicBool>,
    state: Arc<RwLock<HashMap<String, ProfileSnapshot>>>,
    handle: Option<JoinHandle<()>>,
}

impl LocalServer {
    pub fn start(config: ServerConfig, initial: ProfileSnapshot) -> Result<Self, ServerError> {
        if config.token.trim().is_empty() {
            return Err(ServerError::InvalidToken);
        }

        let bind_ip = if config.bind_lan {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };
        let server = Server::http((bind_ip, config.port))
            .map_err(|error| ServerError::Start(error.to_string()))?;
        let addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| ServerError::Start("server is not listening on an IP address".into()))?;

        let server = Arc::new(server);
        let shutdown = Arc::new(AtomicBool::new(false));
        let state = Arc::new(RwLock::new(HashMap::from([(
            config.token.clone(),
            initial,
        )])));
        let token = config.token.clone();

        let handle = {
            let server = Arc::clone(&server);
            let shutdown = Arc::clone(&shutdown);
            let state = Arc::clone(&state);
            thread::Builder::new()
                .name("swiftsub-http".into())
                .spawn(move || {
                    while !shutdown.load(Ordering::Relaxed) {
                        match server.recv_timeout(Duration::from_millis(250)) {
                            Ok(Some(request)) => handle_request(request, &state),
                            Ok(None) => {}
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| ServerError::Start(error.to_string()))?
        };

        Ok(Self {
            addr,
            token,
            server,
            shutdown,
            state,
            handle: Some(handle),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn update_profile(&self, snapshot: ProfileSnapshot) {
        if let Ok(mut state) = self.state.write() {
            state.insert(self.token.clone(), snapshot);
        }
    }

    pub fn replace_profiles(&self, profiles: HashMap<String, ProfileSnapshot>) {
        if let Ok(mut state) = self.state.write() {
            *state = profiles;
        }
    }

    pub fn profile_url(&self, token: &str, filename: &str) -> String {
        format!(
            "http://127.0.0.1:{}/sub/{}/{}",
            self.addr.port(),
            token,
            filename.trim_start_matches('/')
        )
    }

    pub fn url(&self) -> String {
        self.url_for("clash.yaml")
    }

    pub fn url_for(&self, filename: &str) -> String {
        format!(
            "http://127.0.0.1:{}/sub/{}/{}",
            self.addr.port(),
            self.token,
            filename.trim_start_matches('/')
        )
    }

    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.server.unblock();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn is_valid_profile_filename(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\'])
        && (name.ends_with(".yaml") || name.ends_with(".yml"))
}

fn handle_request(request: Request, state: &Arc<RwLock<HashMap<String, ProfileSnapshot>>>) {
    if request.method() != &Method::Get && request.method() != &Method::Head {
        let _ = request.respond(Response::empty(StatusCode(405)));
        return;
    }

    let path = request.url().split('?').next().unwrap_or_default();
    if path == "/health" {
        let _ = request.respond(Response::from_string("ok"));
        return;
    }

    let Some(rest) = path.strip_prefix("/sub/") else {
        let _ = request.respond(Response::empty(StatusCode(404)));
        return;
    };
    let (token, suffix) = rest
        .split_once('/')
        .map(|(t, f)| (t, format!("/{f}")))
        .unwrap_or((rest, String::new()));
    let valid_suffix = if matches!(suffix.as_str(), "" | "/") {
        true
    } else {
        suffix
            .strip_prefix('/')
            .is_some_and(is_valid_profile_filename)
    };
    if !valid_suffix {
        let _ = request.respond(Response::empty(StatusCode(404)));
        return;
    }

    let snapshot = match state.read() {
        Ok(profiles) => match profiles.get(token) {
            Some(snapshot) => snapshot.clone(),
            None => {
                let _ = request.respond(Response::empty(StatusCode(404)));
                return;
            }
        },
        Err(_) => {
            let _ = request.respond(Response::empty(StatusCode(503)));
            return;
        }
    };
    if snapshot.yaml.trim().is_empty() {
        let _ = request.respond(Response::empty(StatusCode(503)));
        return;
    }

    let etag = snapshot.etag();
    if let Some(header) = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("If-None-Match"))
        && header.value.as_str() == etag
    {
        let _ = request.respond(Response::empty(StatusCode(304)));
        return;
    }

    let body = if request.method() == &Method::Head {
        String::new()
    } else {
        snapshot.yaml.clone()
    };

    let mut response = Response::from_string(body).with_status_code(StatusCode(200));
    for (name, value) in [
        ("Content-Type", "text/yaml; charset=utf-8".to_string()),
        ("Cache-Control", "no-store".to_string()),
        ("ETag", etag),
        (
            "Profile-Update-Interval",
            snapshot.update_interval_hours.max(1).to_string(),
        ),
    ] {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(header);
        }
    }
    if let Some(userinfo) = &snapshot.userinfo_header
        && let Ok(header) =
            Header::from_bytes("Subscription-Userinfo".as_bytes(), userinfo.as_bytes())
    {
        response.add_header(header);
    }

    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    use super::*;

    fn request(addr: SocketAddr, path: &str, extra_headers: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{extra_headers}\r\n"
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn serves_profile_with_headers_and_etag() {
        let config = ServerConfig {
            port: 0,
            token: "test-token".into(),
            bind_lan: false,
            update_interval_hours: 12,
        };
        let snapshot = ProfileSnapshot {
            yaml: "proxies: []\nrules:\n  - MATCH,DIRECT\n".into(),
            userinfo_header: Some("upload=1; download=2; total=3".into()),
            update_interval_hours: 12,
        };
        let server = LocalServer::start(config, snapshot).unwrap();
        let addr = server.local_addr();

        let response = request(addr, "/sub/test-token/clash.yaml", "");
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("text/yaml"));
        assert!(response.contains("Subscription-Userinfo: upload=1; download=2; total=3"));
        assert!(response.contains("MATCH,DIRECT"));

        let response = request(addr, "/sub/wrong-token/clash.yaml", "");
        assert!(response.starts_with("HTTP/1.1 404"));

        let response = request(addr, "/health", "");
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.ends_with("ok"));

        let etag = response_etag(&request(addr, "/sub/test-token/clash.yaml", ""));
        let response = request(
            addr,
            "/sub/test-token/clash.yaml",
            &format!("If-None-Match: {etag}\r\n"),
        );
        assert!(response.starts_with("HTTP/1.1 304"));
    }

    #[test]
    fn profiles_remain_isolated_when_active_profile_changes_or_is_deleted() {
        let server = LocalServer::start(
            ServerConfig {
                port: 0,
                token: "one".into(),
                bind_lan: false,
                update_interval_hours: 24,
            },
            ProfileSnapshot::default(),
        )
        .unwrap();
        server.replace_profiles(HashMap::from([
            (
                "one".into(),
                ProfileSnapshot {
                    yaml: "first".into(),
                    ..Default::default()
                },
            ),
            (
                "two".into(),
                ProfileSnapshot {
                    yaml: "second".into(),
                    ..Default::default()
                },
            ),
        ]));
        assert!(request(server.local_addr(), "/sub/one/a.yaml", "").ends_with("first"));
        assert!(request(server.local_addr(), "/sub/two/a.yaml", "").ends_with("second"));
        assert!(
            request(server.local_addr(), "/sub/one-extra/a.yaml", "").starts_with("HTTP/1.1 404")
        );
        server.replace_profiles(HashMap::from([("two".into(), ProfileSnapshot::default())]));
        assert!(request(server.local_addr(), "/sub/one/a.yaml", "").starts_with("HTTP/1.1 404"));
        assert!(request(server.local_addr(), "/sub/two/a.yaml", "").starts_with("HTTP/1.1 503"));
    }

    fn response_etag(response: &str) -> String {
        response
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("etag")
                    .then(|| value.trim().to_string())
            })
            .unwrap()
    }
}
