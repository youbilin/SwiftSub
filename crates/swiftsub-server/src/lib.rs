use std::collections::HashMap;
use std::io::Cursor;
#[cfg(not(windows))]
use std::net::UdpSocket;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const WORKER_COUNT: usize = 4;
const REQUEST_QUEUE_CAPACITY: usize = 32;

use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub port: u16,
    pub token: String,
    pub bind_lan: bool,
    pub lan_private_only: bool,
    pub update_interval_hours: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 7899,
            token: String::new(),
            bind_lan: false,
            lan_private_only: true,
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

struct PublishedProfile {
    yaml: Arc<[u8]>,
    etag: String,
    userinfo_header: Option<String>,
    update_interval_hours: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfilePull {
    pub at: i64,
    pub etag: String,
    pub requests: u64,
}
type PullState = Arc<RwLock<HashMap<String, ProfilePull>>>;

impl From<ProfileSnapshot> for PublishedProfile {
    fn from(snapshot: ProfileSnapshot) -> Self {
        Self {
            etag: snapshot.etag(),
            yaml: Arc::from(snapshot.yaml.into_bytes()),
            userinfo_header: snapshot.userinfo_header,
            update_interval_hours: snapshot.update_interval_hours,
        }
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
    bind_lan: bool,
    lan_private_only: bool,
    token: String,
    server: Arc<Server>,
    shutdown: Arc<AtomicBool>,
    state: Arc<RwLock<HashMap<String, Arc<PublishedProfile>>>>,
    pulls: PullState,
    acceptor: Option<JoinHandle<()>>,
    workers: Vec<JoinHandle<()>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanInterface {
    pub name: String,
    pub description: String,
    pub ip: Ipv4Addr,
    pub has_gateway: bool,
    pub is_virtual: bool,
}

/// Returns true for IPv4 addresses that cannot be reached from LAN peers,
/// including loopback, link-local (APIPA), multicast/broadcast, and RFC 2544
/// `198.18.0.0/15` (used by Clash/Mihomo/sing-box TUN & Fake-IP interfaces).
pub fn is_disallowed_lan_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || (a == 198 && (b == 18 || b == 19))
}

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (64..=127).contains(&b)
}

/// Returns true when an incoming peer IP belongs to loopback, RFC 1918 private LAN,
/// link-local, CGNAT/Tailscale (`100.64.0.0/10`), or IPv6 ULA/link-local space.
pub fn is_private_or_local_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || is_cgnat(v4)
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_private_or_local_ip(std::net::IpAddr::V4(mapped));
            }
            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
        }
    }
}

fn is_virtual_adapter_name(name: &str, description: &str) -> bool {
    let combined = format!("{} {}", name.to_lowercase(), description.to_lowercase());
    const VIRTUAL_KEYWORDS: &[&str] = &[
        "mihomo",
        "clash",
        "meta tunnel",
        "sing-box",
        "singbox",
        "wintun",
        "wireguard",
        "tailscale",
        "zerotier",
        "vmware",
        "vmnet",
        "virtualbox",
        "vbox",
        "hyper-v",
        "vethernet",
        "wsl",
        "docker",
        "wi-fi direct",
        "virtual",
        "tunnel",
        "tap-",
        "tap0",
        "vpn",
        "vnic",
        "bluetooth",
        "loopback",
        "pseudo",
        "npcap",
        "hamachi",
        "radmin",
        "sangfor",
        "openvpn",
        "nekobox",
        "v2ray",
    ];
    VIRTUAL_KEYWORDS
        .iter()
        .any(|keyword| combined.contains(keyword))
}

fn score_lan_interface(iface: &LanInterface, if_type: u32) -> i32 {
    let mut score = 0;
    if !iface.is_virtual {
        score += 1000;
    }
    if iface.has_gateway {
        score += 500;
    }
    if iface.ip.is_private() {
        score += 200;
    }
    if iface.ip == Ipv4Addr::new(192, 168, 137, 1) {
        score -= 250;
    }
    if is_cgnat(iface.ip) {
        score -= 300;
    }
    if if_type == 6 {
        // Ethernet
        score += 20;
    } else if if_type == 71 {
        // IEEE 802.11 Wi-Fi
        score += 10;
    }
    score
}

/// Enumerate active IPv4 network interfaces suitable for LAN subscription import,
/// ranked with physical Ethernet/Wi-Fi adapters with a real gateway first and
/// excluding Clash/Mihomo TUN (`198.18.0.0/15`), loopback, and link-local addresses.
pub fn list_lan_interfaces() -> Vec<LanInterface> {
    #[cfg(windows)]
    {
        list_lan_interfaces_windows()
    }
    #[cfg(not(windows))]
    {
        list_lan_interfaces_fallback()
    }
}

/// Detect the best LAN IPv4 address (preferring physical Wi-Fi/Ethernet over VPN/TUN/VM adapters).
pub fn detect_lan_ipv4() -> Option<Ipv4Addr> {
    list_lan_interfaces().into_iter().next().map(|item| item.ip)
}

#[cfg(windows)]
fn list_lan_interfaces_windows() -> Vec<LanInterface> {
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_INCLUDE_GATEWAYS, GetAdaptersAddresses, IF_TYPE_ETHERNET_CSMACD,
        IF_TYPE_IEEE80211, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::AF_INET;

    let flags = GAA_FLAG_INCLUDE_GATEWAYS;
    let mut size: u32 = 16 * 1024;
    let mut buffer: Vec<u8> = Vec::new();
    let mut succeeded = false;
    for _ in 0..8 {
        buffer = vec![0u8; size as usize];
        let ret = unsafe {
            GetAdaptersAddresses(
                AF_INET as u32,
                flags,
                std::ptr::null(),
                buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
                &mut size,
            )
        };
        if ret == ERROR_SUCCESS {
            succeeded = true;
            break;
        }
        if ret == ERROR_BUFFER_OVERFLOW && size > 0 {
            continue;
        }
        return Vec::new();
    }
    if !succeeded {
        return Vec::new();
    }

    let mut ranked: Vec<(i32, LanInterface)> = Vec::new();
    let mut adapter_ptr = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    while !adapter_ptr.is_null() {
        let adapter = unsafe { &*adapter_ptr };
        adapter_ptr = adapter.Next;

        if adapter.OperStatus != IfOperStatusUp {
            continue;
        }

        let name = unsafe { pwstr_to_string(adapter.FriendlyName) };
        let description = unsafe { pwstr_to_string(adapter.Description) };
        let if_type = adapter.IfType;
        let is_physical_type = if_type == IF_TYPE_ETHERNET_CSMACD || if_type == IF_TYPE_IEEE80211;
        let is_virtual = !is_physical_type || is_virtual_adapter_name(&name, &description);

        let mut has_gateway = false;
        let mut gw_ptr = adapter.FirstGatewayAddress;
        while !gw_ptr.is_null() {
            let gw = unsafe { &*gw_ptr };
            gw_ptr = gw.Next;
            if let Some(gw_ip) = unsafe { sockaddr_to_ipv4(gw.Address.lpSockaddr) }
                && !is_disallowed_lan_ipv4(gw_ip)
            {
                has_gateway = true;
                break;
            }
        }

        let mut unicast_ptr = adapter.FirstUnicastAddress;
        while !unicast_ptr.is_null() {
            let unicast = unsafe { &*unicast_ptr };
            unicast_ptr = unicast.Next;
            let Some(ip) = (unsafe { sockaddr_to_ipv4(unicast.Address.lpSockaddr) }) else {
                continue;
            };
            if is_disallowed_lan_ipv4(ip) {
                continue;
            }
            let iface = LanInterface {
                name: if name.is_empty() {
                    description.clone()
                } else {
                    name.clone()
                },
                description: description.clone(),
                ip,
                has_gateway,
                is_virtual,
            };
            let score = score_lan_interface(&iface, if_type);
            ranked.push((score, iface));
        }
    }

    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.ip.cmp(&b.1.ip)));
    let mut seen = std::collections::HashSet::new();
    ranked
        .into_iter()
        .filter(|(_, iface)| seen.insert(iface.ip))
        .map(|(_, iface)| iface)
        .collect()
}

#[cfg(windows)]
unsafe fn pwstr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf16_lossy(slice)
}

#[cfg(windows)]
unsafe fn sockaddr_to_ipv4(
    ptr: *const windows_sys::Win32::Networking::WinSock::SOCKADDR,
) -> Option<Ipv4Addr> {
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    if ptr.is_null() {
        return None;
    }
    let family = unsafe { (*ptr).sa_family };
    if family != AF_INET {
        return None;
    }
    let sin = unsafe { &*ptr.cast::<SOCKADDR_IN>() };
    let bytes = unsafe { sin.sin_addr.S_un.S_addr }.to_ne_bytes();
    Some(Ipv4Addr::from(bytes))
}

#[cfg(not(windows))]
fn list_lan_interfaces_fallback() -> Vec<LanInterface> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for probe in [
        "192.168.1.1:80",
        "10.0.0.1:80",
        "172.16.0.1:80",
        "223.5.5.5:80",
        "8.8.8.8:80",
    ] {
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if socket.connect(probe).is_ok()
            && let Ok(SocketAddr::V4(addr)) = socket.local_addr()
        {
            let ip = *addr.ip();
            if !is_disallowed_lan_ipv4(ip) && seen.insert(ip) {
                result.push(LanInterface {
                    name: "LAN".into(),
                    description: String::new(),
                    ip,
                    has_gateway: true,
                    is_virtual: false,
                });
            }
        }
    }
    result
}

#[derive(Clone)]
pub struct ProfilePublisher {
    state: Arc<RwLock<HashMap<String, Arc<PublishedProfile>>>>,
    pulls: PullState,
}

impl ProfilePublisher {
    pub fn profile_pull(&self, token: &str) -> Option<ProfilePull> {
        self.pulls.read().ok()?.get(token).cloned()
    }

    pub fn replace_profiles(&self, profiles: HashMap<String, ProfileSnapshot>) {
        let profiles = profiles
            .into_iter()
            .map(|(token, snapshot)| (token, Arc::new(snapshot.into())))
            .collect();
        if let Ok(mut state) = self.state.write() {
            *state = profiles;
            if let Ok(mut pulls) = self.pulls.write() {
                pulls.retain(|token, _| state.contains_key(token));
            }
        }
    }
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
            Arc::new(PublishedProfile::from(initial)),
        )])));
        let token = config.token.clone();
        let pulls: PullState = Arc::new(RwLock::new(HashMap::new()));

        let enforce_private = config.bind_lan && config.lan_private_only;
        let (tx, rx) = mpsc::sync_channel::<Request>(REQUEST_QUEUE_CAPACITY);
        let shared_rx = Arc::new(Mutex::new(rx));
        let mut workers = Vec::with_capacity(WORKER_COUNT);
        for index in 0..WORKER_COUNT {
            let rx = Arc::clone(&shared_rx);
            let state = Arc::clone(&state);
            let pulls = Arc::clone(&pulls);
            let worker = thread::Builder::new()
                .name(format!("swiftsub-http-{index}"))
                .spawn(move || {
                    loop {
                        let next = {
                            let guard = rx.lock().unwrap_or_else(|poison| poison.into_inner());
                            guard.recv()
                        };
                        match next {
                            Ok(request) => handle_request(request, &state, &pulls, enforce_private),
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| ServerError::Start(error.to_string()))?;
            workers.push(worker);
        }

        let acceptor = {
            let server = Arc::clone(&server);
            let shutdown = Arc::clone(&shutdown);
            thread::Builder::new()
                .name("swiftsub-http-accept".into())
                .spawn(move || {
                    while !shutdown.load(Ordering::Relaxed) {
                        match server.recv_timeout(Duration::from_millis(250)) {
                            Ok(Some(request)) => match tx.try_send(request) {
                                Ok(()) => {}
                                Err(TrySendError::Full(request)) => {
                                    let _ = request.respond(Response::empty(StatusCode(503)));
                                }
                                Err(TrySendError::Disconnected(_)) => break,
                            },
                            Ok(None) => {}
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| ServerError::Start(error.to_string()))?
        };

        Ok(Self {
            addr,
            bind_lan: config.bind_lan,
            lan_private_only: config.lan_private_only,
            token,
            server,
            shutdown,
            state,
            pulls,
            acceptor: Some(acceptor),
            workers,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn is_lan_enabled(&self) -> bool {
        self.bind_lan
    }

    pub fn is_lan_private_only(&self) -> bool {
        self.lan_private_only
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn update_profile(&self, snapshot: ProfileSnapshot) {
        if let Ok(mut state) = self.state.write() {
            state.insert(self.token.clone(), Arc::new(snapshot.into()));
        }
    }

    pub fn replace_profiles(&self, profiles: HashMap<String, ProfileSnapshot>) {
        self.publisher().replace_profiles(profiles);
    }

    pub fn publisher(&self) -> ProfilePublisher {
        ProfilePublisher {
            state: Arc::clone(&self.state),
            pulls: Arc::clone(&self.pulls),
        }
    }

    pub fn profile_pulls(&self) -> HashMap<String, ProfilePull> {
        self.pulls
            .read()
            .map(|pulls| pulls.clone())
            .unwrap_or_default()
    }

    pub fn profile_pull(&self, token: &str) -> Option<ProfilePull> {
        self.pulls.read().ok()?.get(token).cloned()
    }

    pub fn published_etag(&self, token: &str) -> Option<String> {
        self.state
            .read()
            .ok()?
            .get(token)
            .map(|profile| profile.etag.clone())
    }
    pub fn published_matches(&self, token: &str, yaml: &str) -> bool {
        self.state.read().is_ok_and(|profiles| {
            profiles
                .get(token)
                .is_some_and(|profile| profile.yaml.as_ref() == yaml.as_bytes())
        })
    }

    pub fn profile_url(&self, token: &str, filename: &str) -> String {
        self.profile_url_for_host("127.0.0.1", token, filename)
    }

    pub fn profile_url_for_host(&self, host: &str, token: &str, filename: &str) -> String {
        format!(
            "http://{}:{}/sub/{}/{}",
            host,
            self.addr.port(),
            token,
            filename.trim_start_matches('/')
        )
    }

    pub fn lan_profile_url(&self, token: &str, filename: &str) -> Option<String> {
        self.lan_profile_url_with_host(None, token, filename)
    }

    pub fn lan_profile_url_with_host(
        &self,
        preferred_host: Option<&str>,
        token: &str,
        filename: &str,
    ) -> Option<String> {
        if !self.bind_lan {
            return None;
        }
        if let Some(host) = preferred_host.map(str::trim).filter(|h| !h.is_empty()) {
            return Some(self.profile_url_for_host(host, token, filename));
        }
        let ip = detect_lan_ipv4()?;
        Some(self.profile_url_for_host(&ip.to_string(), token, filename))
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
        if let Some(acceptor) = self.acceptor.take() {
            let _ = acceptor.join();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
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
        && !name.contains("..")
        && (name.ends_with(".yaml") || name.ends_with(".yml"))
}

fn handle_request(
    request: Request,
    state: &Arc<RwLock<HashMap<String, Arc<PublishedProfile>>>>,
    pulls: &PullState,
    enforce_private: bool,
) {
    if enforce_private
        && !request
            .remote_addr()
            .is_some_and(|peer| is_private_or_local_ip(peer.ip()))
    {
        let _ = request.respond(Response::empty(StatusCode(403)));
        return;
    }
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
    if snapshot.yaml.iter().all(u8::is_ascii_whitespace) {
        let _ = request.respond(Response::empty(StatusCode(503)));
        return;
    }

    let etag = snapshot.etag.clone();
    // A successful GET or conditional GET is a pull; HEAD and internal probes are not.
    let track = request.method() == &Method::Get
        && !request.headers().iter().any(|header| {
            header.field.equiv("User-Agent")
                && header
                    .value
                    .as_str()
                    .starts_with("SwiftSub-Internal-Probe/")
        });
    let pulled_token = token.to_string();
    let record_pull = || {
        if track
            && let Ok(profiles) = state.read()
            && profiles.contains_key(&pulled_token)
            && let Ok(mut pulls) = pulls.write()
        {
            let requests = pulls
                .get(&pulled_token)
                .map_or(1, |pull| pull.requests.saturating_add(1));
            pulls.insert(
                pulled_token.clone(),
                ProfilePull {
                    at: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .min(i64::MAX as u64) as i64,
                    etag: snapshot.etag.clone(),
                    requests,
                },
            );
        }
    };
    if let Some(header) = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("If-None-Match"))
        && header.value.as_str().split(',').any(|value| {
            let value = value.trim();
            value == "*" || value.strip_prefix("W/").unwrap_or(value) == etag
        })
    {
        let mut response = Response::empty(StatusCode(304));
        for (name, value) in [
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
        if request.respond(response).is_ok() {
            record_pull();
        }
        return;
    }

    // tiny_http suppresses the body for HEAD, retaining the GET representation length.
    let mut response = Response::new(
        StatusCode(200),
        Vec::new(),
        Cursor::new(Arc::clone(&snapshot.yaml)),
        Some(snapshot.yaml.len()),
        None,
    );
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

    if request.respond(response).is_ok() {
        record_pull();
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    use super::*;
    #[test]
    fn pulls_exclude_probes_heads_and_errors_and_track_each_published_version() {
        let server = LocalServer::start(
            ServerConfig {
                port: 0,
                token: "one".into(),
                ..Default::default()
            },
            ProfileSnapshot {
                yaml: "first".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let addr = server.local_addr();
        request_method(addr, "HEAD", "/sub/one/a.yaml", "");
        request(addr, "/health", "");
        request(addr, "/sub/unknown/a.yaml", "");
        request(addr, "/sub/one/a.txt", "");
        request(
            addr,
            "/sub/one/a.yaml",
            "User-Agent: SwiftSub-Internal-Probe/1\r\n",
        );
        assert!(server.profile_pulls().is_empty());
        let first_etag = server.published_etag("one").unwrap();
        request(addr, "/sub/one/a.yaml", "");
        wait_for_pull(&server, "one", &first_etag);
        assert_eq!(server.profile_pulls()["one"].etag, first_etag);
        assert!(server.profile_pulls()["one"].at > 0);
        server.update_profile(ProfileSnapshot {
            yaml: "second".into(),
            ..Default::default()
        });
        let second_etag = server.published_etag("one").unwrap();
        assert_ne!(server.profile_pulls()["one"].etag, second_etag);
        assert!(
            request(
                addr,
                "/sub/one/a.yaml",
                &format!("If-None-Match: {second_etag}\r\n")
            )
            .starts_with("HTTP/1.1 304")
        );
        wait_for_pull(&server, "one", &second_etag);
        assert_eq!(server.profile_pulls()["one"].etag, second_etag);
        server.replace_profiles(HashMap::from([(
            "two".into(),
            ProfileSnapshot {
                yaml: "other".into(),
                ..Default::default()
            },
        )]));
        assert!(!server.profile_pulls().contains_key("one"));
        request(addr, "/sub/two/a.yaml", "");
        wait_for_pull(&server, "two", &server.published_etag("two").unwrap());
        assert_eq!(server.profile_pulls().len(), 1);
        server.update_profile(ProfileSnapshot::default());
        assert!(request(addr, "/sub/one/a.yaml", "").starts_with("HTTP/1.1 503"));
        assert!(!server.profile_pulls().contains_key("one"));
    }

    // The response closes before the worker records its successful delivery. Wait for that observable event.
    fn wait_for_pull(server: &LocalServer, token: &str, etag: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if server
                .profile_pulls()
                .get(token)
                .is_some_and(|pull| pull.etag == etag)
            {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "successful pull was not recorded"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn request(addr: SocketAddr, path: &str, extra_headers: &str) -> String {
        request_method(addr, "GET", path, extra_headers)
    }

    fn request_method(addr: SocketAddr, method: &str, path: &str, extra_headers: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{extra_headers}\r\n"
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
            ..Default::default()
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
        assert_eq!(response_etag(&response), etag);

        let head = request_method(addr, "HEAD", "/sub/test-token/clash.yaml", "");
        assert!(head.starts_with("HTTP/1.1 200"));
        assert!(head.ends_with("\r\n\r\n"));
        assert!(head.to_lowercase().contains(&format!(
            "content-length: {}",
            "proxies: []\nrules:\n  - MATCH,DIRECT\n".len()
        )));
        for value in [format!("\"unrelated\", W/{etag}"), "*".into()] {
            let response = request(
                addr,
                "/sub/test-token/clash.yaml",
                &format!("If-None-Match: {value}\r\n"),
            );
            assert!(response.starts_with("HTTP/1.1 304"));
        }
    }

    #[test]
    fn profiles_remain_isolated_when_active_profile_changes_or_is_deleted() {
        let server = LocalServer::start(
            ServerConfig {
                port: 0,
                token: "one".into(),
                bind_lan: false,
                update_interval_hours: 24,
                ..Default::default()
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

    #[test]
    fn lan_binding_listens_on_wildcard_and_formats_lan_urls() {
        let server = LocalServer::start(
            ServerConfig {
                port: 0,
                token: "lan-token".into(),
                bind_lan: true,
                update_interval_hours: 24,
                ..Default::default()
            },
            ProfileSnapshot {
                yaml: "proxies: []\nrules:\n  - MATCH,DIRECT\n".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(server.is_lan_enabled());
        assert_eq!(
            server.local_addr().ip(),
            std::net::IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        );
        let port = server.local_addr().port();
        assert_eq!(
            server.profile_url_for_host("192.168.1.50", "lan-token", "lan.yaml"),
            format!("http://192.168.1.50:{port}/sub/lan-token/lan.yaml")
        );
        let loopback = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        let response = request(loopback, "/sub/lan-token/lan.yaml", "");
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("MATCH,DIRECT"));
    }

    #[test]
    fn lan_detection_excludes_tun_fakeip_and_ranks_physical_adapter_first() {
        assert!(is_disallowed_lan_ipv4(Ipv4Addr::new(198, 18, 0, 1)));
        assert!(is_disallowed_lan_ipv4(Ipv4Addr::new(198, 19, 255, 254)));
        assert!(is_disallowed_lan_ipv4(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(is_disallowed_lan_ipv4(Ipv4Addr::new(169, 254, 41, 45)));
        assert!(!is_disallowed_lan_ipv4(Ipv4Addr::new(10, 160, 148, 44)));
        assert!(!is_disallowed_lan_ipv4(Ipv4Addr::new(192, 168, 1, 20)));

        let wifi = LanInterface {
            name: "WLAN".into(),
            description: "Intel(R) Wi-Fi 6 AX201".into(),
            ip: Ipv4Addr::new(10, 160, 148, 44),
            has_gateway: true,
            is_virtual: false,
        };
        let vmnet = LanInterface {
            name: "VMware Network Adapter VMnet8".into(),
            description: "VMware Virtual Ethernet Adapter for VMnet8".into(),
            ip: Ipv4Addr::new(192, 168, 88, 1),
            has_gateway: false,
            is_virtual: true,
        };
        let tailscale = LanInterface {
            name: "Tailscale".into(),
            description: "Tailscale Tunnel".into(),
            ip: Ipv4Addr::new(100, 116, 137, 9),
            has_gateway: false,
            is_virtual: true,
        };
        assert!(score_lan_interface(&wifi, 71) > score_lan_interface(&vmnet, 6));
        assert!(score_lan_interface(&wifi, 71) > score_lan_interface(&tailscale, 53));
        assert!(is_virtual_adapter_name("Mihomo", "Meta Tunnel"));
        assert!(is_virtual_adapter_name(
            "VMware Network Adapter VMnet8",
            "VMware Virtual Ethernet Adapter"
        ));
        assert!(!is_virtual_adapter_name(
            "WLAN",
            "Intel(R) Wi-Fi 6 AX201 160MHz"
        ));
        if let Some(detected) = detect_lan_ipv4() {
            assert!(!is_disallowed_lan_ipv4(detected));
        }
    }

    #[test]
    fn concurrent_requests_and_slow_client_do_not_block_other_clients() {
        let server = LocalServer::start(
            ServerConfig {
                port: 0,
                token: "pool-token".into(),
                bind_lan: false,
                update_interval_hours: 24,
                ..Default::default()
            },
            ProfileSnapshot {
                yaml: "proxies: []\nrules:\n  - MATCH,DIRECT\n".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let addr = server.local_addr();

        // Hold an incomplete HTTP connection open to simulate a stalled LAN peer.
        let mut slow_peer = TcpStream::connect(addr).unwrap();
        slow_peer
            .write_all(b"GET /sub/pool-token/clash.yaml HTTP/1.1\r\nHost: localhost\r\n")
            .unwrap();

        let handles: Vec<_> = (0..8)
            .map(|_| {
                thread::spawn(move || {
                    let response = request(addr, "/sub/pool-token/clash.yaml", "");
                    assert!(response.starts_with("HTTP/1.1 200"));
                    assert!(response.contains("MATCH,DIRECT"));
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        // Reject unsupported methods and traversal filenames.
        assert!(
            request_method(addr, "POST", "/sub/pool-token/clash.yaml", "")
                .starts_with("HTTP/1.1 405")
        );
        assert!(request(addr, "/sub/pool-token/..secret.yaml", "").starts_with("HTTP/1.1 404"));
        assert!(request(addr, "/sub/pool-token/a/b.yaml", "").starts_with("HTTP/1.1 404"));

        drop(slow_peer);
    }

    #[test]
    fn classifies_private_and_public_peer_addresses() {
        use std::net::{IpAddr, Ipv6Addr};
        assert!(is_private_or_local_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_private_or_local_ip(IpAddr::V4(Ipv4Addr::new(
            192, 168, 1, 10
        ))));
        assert!(is_private_or_local_ip(IpAddr::V4(Ipv4Addr::new(
            10, 0, 0, 5
        ))));
        assert!(is_private_or_local_ip(IpAddr::V4(Ipv4Addr::new(
            172, 16, 0, 2
        ))));
        assert!(is_private_or_local_ip(IpAddr::V4(Ipv4Addr::new(
            100, 100, 10, 2
        ))));
        assert!(!is_private_or_local_ip(IpAddr::V4(Ipv4Addr::new(
            8, 8, 8, 8
        ))));
        assert!(!is_private_or_local_ip(IpAddr::V4(Ipv4Addr::new(
            1, 1, 1, 1
        ))));
        assert!(is_private_or_local_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_private_or_local_ip(IpAddr::V6(
            "fd12:3456:789a::1".parse().unwrap()
        )));
        assert!(!is_private_or_local_ip(IpAddr::V6(
            "2606:4700:4700::1111".parse().unwrap()
        )));
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
