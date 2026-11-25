//! Hardened HTTP/HTTPS subscription & ACME server with per-IP concurrency
//! isolation, token-bucket rate limiting, zero-downtime certificate hot-reload,
//! dark Chinese overview dashboard (`/index`), and native Argo loopback mode.

use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rustls_pemfile::{certs, private_key};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio_rustls::TlsAcceptor;

use crate::apply::{self, SERVE_AUTH_REL, ServeAuth};
use crate::compiler::{self, pct};
use crate::config::Config;
use crate::fs_util;
use crate::qr;
use crate::secrets::Secrets;
use crate::traffic::{self, TrafficSnapshot};

const MAX_HEADER_BYTES: usize = 16 * 1024;
const HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT_CONNS: usize = 32;
const MAX_CONNS_PER_IP: usize = 8;
const RATE_BURST: u32 = 60;
const RATE_REFILL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListenerKind {
    DirectHttps,
    FallbackHttp,
    ArgoLoopback,
}

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    kind: ListenerKind,
    budget: Arc<Mutex<IpBudget>>,
}

struct ConnectionLimits {
    global: Arc<Semaphore>,
    peers: Arc<Mutex<HashMap<IpAddr, usize>>>,
    per_peer: bool,
}

impl ConnectionLimits {
    fn new(per_peer: bool) -> Self {
        Self {
            global: Arc::new(Semaphore::new(MAX_CONCURRENT_CONNS)),
            peers: Arc::new(Mutex::new(HashMap::new())),
            per_peer,
        }
    }

    fn try_acquire(&self, peer: Option<IpAddr>) -> Option<ConnPermit> {
        let global = self.global.clone().try_acquire_owned().ok()?;
        let peer = peer.filter(|_| self.per_peer).map(|ip| ip.to_canonical());
        if let Some(ip) = peer {
            let mut map = self.peers.lock().unwrap_or_else(|e| e.into_inner());
            let count = map.entry(ip).or_default();
            if *count >= MAX_CONNS_PER_IP {
                return None;
            }
            *count += 1;
        }
        Some(ConnPermit {
            _global: global,
            peers: Arc::clone(&self.peers),
            peer,
        })
    }
}

struct ConnPermit {
    _global: OwnedSemaphorePermit,
    peers: Arc<Mutex<HashMap<IpAddr, usize>>>,
    peer: Option<IpAddr>,
}

impl Drop for ConnPermit {
    fn drop(&mut self) {
        if let Some(ip) = self.peer {
            let mut map = self.peers.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(count) = map.get_mut(&ip) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    map.remove(&ip);
                }
            }
        }
    }
}

struct Bucket {
    tokens: u32,
    at: Instant,
}

struct IpBudget {
    buckets: HashMap<IpAddr, Bucket>,
}

impl IpBudget {
    fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    fn acquire(&mut self, peer: IpAddr, now: Instant) -> Result<(), Duration> {
        let Some(b) = self.buckets.get_mut(&peer) else {
            if self.buckets.len() > 4096 {
                self.buckets.clear();
            }
            self.buckets.insert(
                peer,
                Bucket {
                    tokens: RATE_BURST - 1,
                    at: now,
                },
            );
            return Ok(());
        };
        let elapsed = now.saturating_duration_since(b.at);
        let refill = (elapsed.as_nanos() / RATE_REFILL.as_nanos()) as u32;
        if refill > 0 {
            b.tokens = b.tokens.saturating_add(refill).min(RATE_BURST);
            b.at =
                b.at.checked_add(RATE_REFILL.saturating_mul(refill))
                    .unwrap_or(now);
        }
        if b.tokens == 0 {
            return Err(Duration::from_secs(1));
        }
        b.tokens -= 1;
        Ok(())
    }
}

pub async fn run_server(root: PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::load(&root)?;
    let _ = traffic::checkpoint(&root, &config);

    let budget = Arc::new(Mutex::new(IpBudget::new()));
    let mut handles = Vec::new();

    // Background traffic checkpoint every 60s
    let ckpt_root = root.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Ok(cfg) = Config::load(&ckpt_root) {
                let _ = traffic::checkpoint(&ckpt_root, &cfg);
            }
        }
    });

    if config.is_ip_mode() {
        let addr: SocketAddr = ([0, 0, 0, 0], config.server.http_fallback_port).into();
        let listener = TcpListener::bind(addr).await?;
        eprintln!("vps-sub-meter listening HTTP fallback on {addr}");
        let state = AppState {
            root: root.clone(),
            kind: ListenerKind::FallbackHttp,
            budget: Arc::clone(&budget),
        };
        handles.push(tokio::spawn(async move {
            serve_http_loop(listener, state, true).await;
        }));
    } else {
        // ACME HTTP-01 listener
        let acme_addr: SocketAddr = ([0, 0, 0, 0], config.server.acme_port).into();
        let acme_listener = TcpListener::bind(acme_addr).await?;
        let acme_root = root.clone();
        eprintln!("vps-sub-meter listening ACME HTTP on {acme_addr}");
        handles.push(tokio::spawn(async move {
            serve_acme_loop(acme_listener, acme_root).await;
        }));

        // TLS reloader
        let (tls_tx, tls_rx) = watch::channel(None);
        let tls_root = root.clone();
        let tls_sni = config.tls_sni().to_owned();
        tokio::spawn(async move {
            watch_tls_material(tls_root, tls_sni, tls_tx).await;
        });

        // Primary HTTPS listener (loopback when Xray fronts 443, else 0.0.0.0:https_port)
        let primary_addr: SocketAddr = if config.xray_fronts_443() {
            ([127, 0, 0, 1], config.server.loopback_https_port).into()
        } else {
            ([0, 0, 0, 0], config.server.https_port).into()
        };
        let primary_listener = TcpListener::bind(primary_addr).await?;
        eprintln!("vps-sub-meter listening HTTPS on {primary_addr}");
        let primary_state = AppState {
            root: root.clone(),
            kind: ListenerKind::DirectHttps,
            budget: Arc::clone(&budget),
        };
        let rx1 = tls_rx.clone();
        let per_peer = !config.xray_fronts_443();
        handles.push(tokio::spawn(async move {
            serve_https_loop(primary_listener, primary_state, rx1, per_peer).await;
        }));

        // Optional extra HTTPS port (e.g. 8443 compatibility during migration)
        if let Some(extra) = config.server.extra_https_port {
            let extra_addr: SocketAddr = ([0, 0, 0, 0], extra).into();
            let extra_listener = TcpListener::bind(extra_addr).await?;
            eprintln!("vps-sub-meter listening extra HTTPS on {extra_addr}");
            let extra_state = AppState {
                root: root.clone(),
                kind: ListenerKind::DirectHttps,
                budget: Arc::clone(&budget),
            };
            let rx2 = tls_rx.clone();
            handles.push(tokio::spawn(async move {
                serve_https_loop(extra_listener, extra_state, rx2, true).await;
            }));
        }
    }

    // Native Cloudflare Argo loopback HTTP listener
    if config.argo.enabled {
        let argo_addr: SocketAddr = ([127, 0, 0, 1], config.argo.loopback_sub_port).into();
        let argo_listener = TcpListener::bind(argo_addr).await?;
        eprintln!("vps-sub-meter listening Argo loopback HTTP on {argo_addr}");
        let argo_state = AppState {
            root: root.clone(),
            kind: ListenerKind::ArgoLoopback,
            budget: Arc::clone(&budget),
        };
        handles.push(tokio::spawn(async move {
            serve_http_loop(argo_listener, argo_state, false).await;
        }));
    }

    for h in handles {
        let _ = h.await;
    }
    Ok(())
}

async fn watch_tls_material(
    root: PathBuf,
    sni: String,
    tx: watch::Sender<Option<Arc<rustls::ServerConfig>>>,
) {
    let dir = root.join("var/lib/vps-sub-meter/certificates").join(&sni);
    let cert_path = dir.join("fullchain.pem");
    let key_path = dir.join("privkey.pem");
    let mut last_stamp: Option<(SystemTime, u64, SystemTime, u64)> = None;
    loop {
        let stamp = (
            fs::metadata(&cert_path)
                .ok()
                .and_then(|m| Some((m.modified().ok()?, m.len()))),
            fs::metadata(&key_path)
                .ok()
                .and_then(|m| Some((m.modified().ok()?, m.len()))),
        );
        if let (Some((cm, cl)), Some((km, kl))) = stamp {
            let cur = (cm, cl, km, kl);
            if last_stamp != Some(cur)
                && let Ok(cfg) = load_rustls_config(&cert_path, &key_path)
            {
                tx.send_replace(Some(Arc::new(cfg)));
                last_stamp = Some(cur);
                eprintln!("TLS certificate loaded/reloaded for {sni}");
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn load_rustls_config(
    cert_path: &Path,
    key_path: &Path,
) -> Result<rustls::ServerConfig, Box<dyn std::error::Error + Send + Sync>> {
    let cert_chain =
        certs(&mut BufReader::new(File::open(cert_path)?)).collect::<Result<Vec<_>, _>>()?;
    let key = private_key(&mut BufReader::new(File::open(key_path)?))?.ok_or("私钥文件为空")?;
    let mut cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(cfg)
}

fn http1_builder() -> hyper::server::conn::http1::Builder {
    let mut b = hyper::server::conn::http1::Builder::new();
    b.max_buf_size(MAX_HEADER_BYTES);
    b.timer(hyper_util::rt::TokioTimer::new());
    b.header_read_timeout(HEADER_TIMEOUT);
    b
}

async fn serve_acme_loop(listener: TcpListener, root: PathBuf) {
    let limits = ConnectionLimits::new(true);
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let peer = stream.peer_addr().ok().map(|a| a.ip());
        let Some(permit) = limits.try_acquire(peer) else {
            drop(stream);
            continue;
        };
        let r = root.clone();
        tokio::spawn(async move {
            let _p = permit;
            let svc = service_fn(move |req: Request<Incoming>| {
                let resp = with_len(handle_acme(req, &r));
                async { Ok::<_, std::convert::Infallible>(resp) }
            });
            let _ = tokio::time::timeout(
                CONNECTION_TIMEOUT,
                http1_builder().serve_connection(TokioIo::new(stream), svc),
            )
            .await;
        });
    }
}

async fn serve_http_loop(listener: TcpListener, state: AppState, per_peer: bool) {
    let limits = ConnectionLimits::new(per_peer);
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let peer = stream.peer_addr().ok().map(|a| a.ip());
        let Some(permit) = limits.try_acquire(peer) else {
            drop(stream);
            continue;
        };
        let st = state.clone();
        tokio::spawn(async move {
            let _p = permit;
            let svc = service_fn(move |req: Request<Incoming>| {
                let resp = with_len(handle_subscription_request(req, &st, peer));
                async { Ok::<_, std::convert::Infallible>(resp) }
            });
            let _ = tokio::time::timeout(
                CONNECTION_TIMEOUT,
                http1_builder().serve_connection(TokioIo::new(stream), svc),
            )
            .await;
        });
    }
}

async fn serve_https_loop(
    listener: TcpListener,
    state: AppState,
    tls_rx: watch::Receiver<Option<Arc<rustls::ServerConfig>>>,
    per_peer: bool,
) {
    let limits = ConnectionLimits::new(per_peer);
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let peer = stream.peer_addr().ok().map(|a| a.ip());
        let Some(permit) = limits.try_acquire(peer) else {
            drop(stream);
            continue;
        };
        let Some(tls_cfg) = tls_rx.borrow().clone() else {
            drop(stream);
            continue;
        };
        let st = state.clone();
        tokio::spawn(async move {
            let _p = permit;
            let acceptor = TlsAcceptor::from(tls_cfg);
            let Ok(Ok(tls_stream)) =
                tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await
            else {
                return;
            };
            let svc = service_fn(move |req: Request<Incoming>| {
                let resp = with_len(handle_subscription_request(req, &st, peer));
                async { Ok::<_, std::convert::Infallible>(resp) }
            });
            let _ = tokio::time::timeout(
                CONNECTION_TIMEOUT,
                http1_builder().serve_connection(TokioIo::new(Box::pin(tls_stream)), svc),
            )
            .await;
        });
    }
}

fn with_len(mut resp: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    use hyper::body::Body;
    let len = resp.body().size_hint().exact().unwrap_or(0);
    resp.headers_mut()
        .insert(hyper::header::CONTENT_LENGTH, len.into());
    resp
}

fn handle_acme(req: Request<Incoming>, root: &Path) -> Response<Full<Bytes>> {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return plain_status(StatusCode::METHOD_NOT_ALLOWED, b"Method Not Allowed\n");
    }
    let Some(token) = req
        .uri()
        .path()
        .strip_prefix("/.well-known/acme-challenge/")
        .filter(|t| {
            !t.is_empty()
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    else {
        return plain_status(StatusCode::NOT_FOUND, b"Not Found\n");
    };
    let path = root
        .join("var/lib/vps-sub-meter/acme-webroot/.well-known/acme-challenge")
        .join(token);
    match fs_util::read_no_follow(&path) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Cache-Control", "no-store")
            .header("Connection", "close")
            .body(Full::new(Bytes::from(bytes)))
            .unwrap(),
        Err(_) => plain_status(StatusCode::NOT_FOUND, b"Not Found\n"),
    }
}

fn handle_subscription_request(
    req: Request<Incoming>,
    state: &AppState,
    peer: Option<IpAddr>,
) -> Response<Full<Bytes>> {
    route_request(
        req.method(),
        req.uri().path(),
        req.uri().query(),
        state,
        peer,
    )
}

fn route_request(
    method: &Method,
    path: &str,
    query: Option<&str>,
    state: &AppState,
    peer: Option<IpAddr>,
) -> Response<Full<Bytes>> {
    // Public listeners enforce per-IP token bucket; loopback/Xray-forwarded skip IP throttling.
    if state.kind != ListenerKind::ArgoLoopback
        && let Some(ip) = peer
        && !ip.is_loopback()
    {
        let mut budget = state.budget.lock().unwrap_or_else(|e| e.into_inner());
        if budget.acquire(ip, Instant::now()).is_err() {
            return Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header("Retry-After", "1")
                .header("Cache-Control", "no-store")
                .header("Connection", "close")
                .body(Full::new(Bytes::new()))
                .unwrap();
        }
    }

    if !matches!(*method, Method::GET | Method::HEAD) {
        return plain_status(StatusCode::METHOD_NOT_ALLOWED, b"Method Not Allowed\n");
    }
    // Reject all query strings to prevent cache poisoning or parameter overrides.
    if query.is_some() {
        return plain_status(StatusCode::NOT_FOUND, b"Not Found\n");
    }

    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if parts.len() < 2 || parts[0] != "sub" {
        return plain_status(StatusCode::NOT_FOUND, b"Not Found\n");
    }
    let token = parts[1];
    let tail = parts[2..].join("/");

    let Ok(config) = Config::load(&state.root) else {
        return plain_status(StatusCode::SERVICE_UNAVAILABLE, b"Service Unavailable\n");
    };
    let Ok(auth_bytes) = fs_util::read_no_follow(&state.root.join(SERVE_AUTH_REL)) else {
        return plain_status(StatusCode::SERVICE_UNAVAILABLE, b"Service Unavailable\n");
    };
    let Ok(auth) = serde_json::from_slice::<ServeAuth>(&auth_bytes) else {
        return plain_status(StatusCode::SERVICE_UNAVAILABLE, b"Service Unavailable\n");
    };

    if !constant_time_hash_match(&auth.token_sha256, token) {
        return plain_status(StatusCode::NOT_FOUND, b"Not Found\n");
    }

    let snap = traffic::snapshot(&state.root, &config).ok();
    let userinfo = snap.as_ref().map(|s| s.userinfo_header());

    match state.kind {
        ListenerKind::ArgoLoopback => {
            let secrets = Secrets {
                argo_ws: auth.argo_ws,
                ..Default::default()
            };
            let Ok(bundle) = compiler::compile_argo_suite(&state.root, &config, &secrets) else {
                return plain_status(StatusCode::SERVICE_UNAVAILABLE, b"Argo Error\n");
            };
            let base = format!("https://{}/sub/{token}", config.argo.sub_domain.trim());
            serve_bundle_route(
                &tail,
                &config,
                &bundle,
                &base,
                &config.argo.profile_name,
                snap.as_ref(),
                userinfo.as_deref(),
                true,
            )
        }
        ListenerKind::DirectHttps | ListenerKind::FallbackHttp => {
            let Ok(Some(gen_meta)) = apply::current_generation(&state.root) else {
                return plain_status(StatusCode::SERVICE_UNAVAILABLE, b"Not Ready\n");
            };
            let clash_yaml = apply::read_generation_file(&state.root, &gen_meta.id, "clash.yaml")
                .ok()
                .flatten()
                .unwrap_or_default();
            let sing_box_json =
                apply::read_generation_file(&state.root, &gen_meta.id, "sing-box.json")
                    .ok()
                    .flatten();
            let shadowrocket_txt =
                apply::read_generation_file(&state.root, &gen_meta.id, "shadowrocket.txt")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
            let uri_txt = apply::read_generation_file(&state.root, &gen_meta.id, "uri.txt")
                .ok()
                .flatten()
                .unwrap_or_default();
            let bundle = compiler::ClientBundle {
                clash_yaml,
                sing_box_json,
                shadowrocket_txt,
                uri_txt,
                direct_node_names: gen_meta.direct_nodes,
                singbox_node_names: gen_meta.singbox_nodes,
            };
            let base = config.subscription_base_url(token);
            serve_bundle_route(
                &tail,
                &config,
                &bundle,
                &base,
                &config.profile_name,
                snap.as_ref(),
                userinfo.as_deref(),
                false,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn serve_bundle_route(
    tail: &str,
    config: &Config,
    bundle: &compiler::ClientBundle,
    base_url: &str,
    profile_title: &str,
    snap: Option<&TrafficSnapshot>,
    userinfo: Option<&str>,
    is_argo: bool,
) -> Response<Full<Bytes>> {
    let title_b64 = format!("base64:{}", STANDARD.encode(profile_title.as_bytes()));
    let index_url = format!("{base_url}/index");

    match tail {
        "clash.yaml" => build_sub_resp(
            StatusCode::OK,
            "text/yaml; charset=utf-8",
            bundle.clash_yaml.as_bytes(),
            Some("vsm-clash.yaml"),
            Some(&title_b64),
            Some(&index_url),
            userinfo,
            is_argo,
        ),
        "sing-box.json" | "sing-box-full.json" => match &bundle.sing_box_json {
            Some(json_str) => build_sub_resp(
                StatusCode::OK,
                "application/json; charset=utf-8",
                json_str.as_bytes(),
                Some("sing-box.json"),
                Some(&title_b64),
                Some(&index_url),
                userinfo,
                is_argo,
            ),
            None => plain_status(StatusCode::NOT_FOUND, b"No compatible sing-box nodes\n"),
        },
        "shadowrocket.txt" => build_sub_resp(
            StatusCode::OK,
            "text/plain; charset=utf-8",
            bundle.shadowrocket_txt.as_bytes(),
            Some("shadowrocket.txt"),
            Some(&title_b64),
            Some(&index_url),
            userinfo,
            is_argo,
        ),
        "uri.txt" | "uri" => build_sub_resp(
            StatusCode::OK,
            "text/plain; charset=utf-8",
            bundle.uri_txt.as_bytes(),
            None,
            None,
            None,
            userinfo,
            is_argo,
        ),
        "" | "index" | "qr" => {
            let html = build_index_html(config, bundle, base_url, profile_title, snap, is_argo);
            build_sub_resp(
                StatusCode::OK,
                "text/html; charset=utf-8",
                html.as_bytes(),
                None,
                None,
                None,
                None,
                is_argo,
            )
        }
        "qr/clash.svg" | "qr/clash.yaml" => {
            render_qr_response(&format!("{base_url}/clash.yaml"), is_argo)
        }
        "qr/sing-box.svg" | "qr/sing-box.json" => {
            if bundle.sing_box_json.is_none() {
                return plain_status(StatusCode::NOT_FOUND, b"Not Found\n");
            }
            let payload = format!(
                "sing-box://import-remote-profile?url={}#{}",
                pct(&format!("{base_url}/sing-box.json")),
                pct(profile_title)
            );
            render_qr_response(&payload, is_argo)
        }
        "qr/shadowrocket.svg" | "qr/shadowrocket.txt" => {
            render_qr_response(&format!("{base_url}/shadowrocket.txt"), is_argo)
        }
        "qr/shadowrocket-sub.svg" => {
            let sr_sub = format!(
                "sub://{}#{}",
                STANDARD.encode(format!("{base_url}/shadowrocket.txt").as_bytes()),
                pct(profile_title)
            );
            render_qr_response(&sr_sub, is_argo)
        }
        "qr/uri.svg" | "qr/uri.txt" => render_qr_response(&format!("{base_url}/uri.txt"), is_argo),
        _ => plain_status(StatusCode::NOT_FOUND, b"Not Found\n"),
    }
}

fn render_qr_response(payload: &str, is_argo: bool) -> Response<Full<Bytes>> {
    match qr::render_svg(payload) {
        Ok(svg) => build_sub_resp(
            StatusCode::OK,
            "image/svg+xml; charset=utf-8",
            svg.as_bytes(),
            None,
            None,
            None,
            None,
            is_argo,
        ),
        Err(_) => plain_status(StatusCode::INTERNAL_SERVER_ERROR, b"QR error\n"),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_sub_resp(
    status: StatusCode,
    content_type: &str,
    body: &[u8],
    filename: Option<&str>,
    profile_title: Option<&str>,
    web_url: Option<&str>,
    userinfo: Option<&str>,
    is_argo: bool,
) -> Response<Full<Bytes>> {
    let mut b = Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Robots-Tag", "noindex, nofollow")
        .header("Connection", "close");
    if is_argo {
        b = b
            .header("CDN-Cache-Control", "no-store")
            .header("Cloudflare-CDN-Cache-Control", "no-store");
    }
    if let Some(name) = filename {
        b = b.header(
            "Content-Disposition",
            format!("inline; filename=\"{name}\""),
        );
    }
    if let Some(title) = profile_title {
        b = b.header("Profile-Title", title);
        b = b.header("Profile-Update-Interval", "24");
    }
    if let Some(url) = web_url {
        b = b.header("Profile-Web-Page-Url", url);
    }
    if let Some(ui) = userinfo {
        b = b.header("Subscription-Userinfo", ui);
    }
    b.body(Full::new(Bytes::copy_from_slice(body))).unwrap()
}

fn plain_status(status: StatusCode, body: &'static [u8]) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Cache-Control", "no-store")
        .header("X-Robots-Tag", "noindex, nofollow")
        .header("Connection", "close")
        .body(Full::new(Bytes::from_static(body)))
        .unwrap()
}

fn constant_time_hash_match(hashes: &[String], candidate_token: &str) -> bool {
    if candidate_token.is_empty() || candidate_token.len() > 128 {
        return false;
    }
    let cand_hex: String = Sha256::digest(candidate_token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let cand_bytes = cand_hex.as_bytes();
    let mut matched = 0u8;
    for expected in hashes {
        if expected.len() != cand_bytes.len() {
            continue;
        }
        let mut diff = 0u8;
        for (a, b) in expected.as_bytes().iter().zip(cand_bytes.iter()) {
            diff |= a ^ b;
        }
        if diff == 0 {
            matched = 1;
        }
    }
    matched == 1
}

fn build_index_html(
    config: &Config,
    bundle: &compiler::ClientBundle,
    base_url: &str,
    title: &str,
    snap: Option<&TrafficSnapshot>,
    is_argo: bool,
) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    };
    let (used_str, total_str, up_str, down_str, pct_val, reset_str) = match snap {
        Some(s) => {
            let used = s.total_used();
            let pct_f = if s.monthly_limit_bytes > 0 {
                (used as f64 * 100.0 / s.monthly_limit_bytes as f64).min(100.0)
            } else {
                0.0
            };
            let total_lbl = if s.monthly_limit_bytes > 0 {
                traffic::format_iec(s.monthly_limit_bytes)
            } else {
                "不限".into()
            };
            (
                traffic::format_iec(used),
                total_lbl,
                traffic::format_iec(s.upload_bytes),
                traffic::format_iec(s.download_bytes),
                format!("{pct_f:.2}"),
                format!(
                    "{} ({})",
                    traffic::format_time_in_tz(s.next_reset_ts, &config.traffic.display_tz),
                    config.traffic.display_tz
                ),
            )
        }
        None => (
            "暂不可用".into(),
            "-".into(),
            "-".into(),
            "-".into(),
            "0.00".into(),
            "-".into(),
        ),
    };

    let clash_url = format!("{base_url}/clash.yaml");
    let sr_url = format!("{base_url}/shadowrocket.txt");
    let sr_sub_url = format!(
        "sub://{}#{}",
        STANDARD.encode(sr_url.as_bytes()),
        pct(title)
    );
    let uri_url = format!("{base_url}/uri.txt");

    let mut cards = String::new();
    let mut push_card = |card_title: &str, desc: &str, link: &str, qr_payload: &str| {
        let svg = qr::render_svg(qr_payload).unwrap_or_default();
        cards.push_str(&format!(
            r#"<div class="card">
  <h2>{t}</h2>
  <p class="desc">{d}</p>
  <div class="qr">{svg}</div>
  <div class="url-box"><code>{l}</code></div>
  <button type="button" onclick="copyText(this)" data-val="{l}">复制链接</button>
</div>"#,
            t = esc(card_title),
            d = esc(desc),
            l = esc(link)
        ));
    };

    push_card(
        "Clash Party / Mihomo 订阅",
        "支持全部节点及 TCP优选/自动选择组。",
        &clash_url,
        &clash_url,
    );
    if bundle.sing_box_json.is_some() {
        let sb_url = format!("{base_url}/sing-box.json");
        let sb_qr = format!(
            "sing-box://import-remote-profile?url={}#{}",
            pct(&sb_url),
            pct(title)
        );
        push_card(
            &format!(
                "sing-box 1.14+ 订阅 ({} 节点)",
                bundle.singbox_node_names.len()
            ),
            if is_argo {
                "Cloudflare Argo 优选节点（严格 TCP）。"
            } else {
                "包含 AnyTLS / Hy2 跳跃 / TUIC；Xray PQC 节点已自动过滤。"
            },
            &sb_url,
            &sb_qr,
        );
    }
    push_card(
        "Shadowrocket 扫码订阅 (推荐)",
        "使用 Shadowrocket 首页左上角扫码直接添加。",
        &sr_sub_url,
        &sr_sub_url,
    );
    push_card(
        "Shadowrocket / 通用 Base64",
        "粘贴至 Shadowrocket 或 v2rayN 订阅地址。",
        &sr_url,
        &sr_url,
    );
    push_card("明文 URI 列表", "每行一条分享链接。", &uri_url, &uri_url);

    let nodes_summary = esc(&bundle.direct_node_names.join(" · "));
    let uri_escaped = esc(bundle.uri_txt.trim());

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex,nofollow">
<meta name="referrer" content="no-referrer">
<title>{title}</title>
<style>
  body {{ font-family: system-ui, -apple-system, sans-serif; margin: 0; padding: 20px; background: #0f172a; color: #e2e8f0; }}
  .wrap {{ max-width: 1020px; margin: 0 auto; }}
  h1 {{ font-size: 1.4rem; margin: 0 0 8px; }}
  .sub {{ color: #94a3b8; font-size: 0.9rem; margin-bottom: 16px; }}
  .stats {{ background: #1e293b; border: 1px solid #334155; border-radius: 12px; padding: 16px; margin-bottom: 16px; display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 16px; align-items: center; }}
  .stat-label {{ font-size: 0.85rem; color: #94a3b8; margin-bottom: 4px; }}
  .stat-value {{ font-size: 1.3rem; font-weight: 700; color: #f8fafc; }}
  .stat-value span {{ font-size: 0.95rem; font-weight: 500; color: #94a3b8; }}
  .stat-meta {{ font-size: 0.82rem; color: #94a3b8; margin-top: 6px; }}
  .bar {{ height: 8px; background: #0f172a; border-radius: 999px; overflow: hidden; margin-top: 8px; }}
  .bar-fill {{ height: 100%; background: linear-gradient(90deg, #38bdf8, #3b82f6); width: {pct_val}%; }}
  .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 16px; }}
  .card {{ background: #1e293b; border: 1px solid #334155; border-radius: 12px; padding: 16px; display: flex; flex-direction: column; gap: 10px; }}
  .card h2 {{ font-size: 1.02rem; margin: 0; color: #f8fafc; }}
  .desc {{ font-size: 0.84rem; color: #94a3b8; margin: 0; line-height: 1.4; }}
  .qr {{ background: #fff; border-radius: 8px; padding: 10px; align-self: center; }}
  .qr svg {{ display: block; width: 180px; height: 180px; }}
  .url-box {{ background: #0f172a; border: 1px solid #334155; border-radius: 6px; padding: 8px; overflow-x: auto; }}
  code {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.78rem; word-break: break-all; color: #38bdf8; }}
  button {{ cursor: pointer; background: #2563eb; color: #fff; border: none; border-radius: 6px; padding: 8px 12px; font-size: 0.9rem; }}
  button:hover {{ background: #1d4ed8; }}
</style>
</head>
<body>
<div class="wrap">
  <h1>{title}</h1>
  <p class="sub">{nodes_summary}</p>
  <div class="stats">
    <div>
      <div class="stat-label">本期流量</div>
      <div class="stat-value">{used_str} <span>/ {total_str}</span></div>
      <div class="bar"><div class="bar-fill"></div></div>
      <div class="stat-meta">已用 {pct_val}% · 上传 {up_str} · 下载 {down_str}</div>
    </div>
    <div>
      <div class="stat-label">下次重置</div>
      <div class="stat-value">{reset_str}</div>
      <div class="stat-meta">共 {node_count} 个节点</div>
    </div>
  </div>
  <div class="grid">{cards}</div>
  <div class="card" style="margin-top:16px">
    <h2>节点分享链接原文</h2>
    <div class="url-box"><code style="white-space:pre-wrap">{uri_escaped}</code></div>
    <button type="button" onclick="copyText(this)" data-val="{uri_escaped}">复制全部节点链接</button>
  </div>
</div>
<script>
function copyText(btn) {{
  const val = btn.getAttribute("data-val");
  navigator.clipboard.writeText(val).then(() => {{
    const old = btn.textContent;
    btn.textContent = "已复制";
    setTimeout(() => btn.textContent = old, 1500);
  }});
}}
</script>
</body>
</html>"#,
        title = esc(title),
        node_count = bundle.direct_node_names.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::body::Body;

    #[test]
    fn rejects_invalid_tokens_and_query_params_and_serves_valid_routes() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..Default::default()
        };
        cfg.save(dir.path()).unwrap();
        let sec = Secrets::generate_for_test(&cfg);
        sec.save(dir.path()).unwrap();
        apply::apply_all(dir.path()).unwrap();

        let state = AppState {
            root: dir.path().to_path_buf(),
            kind: ListenerKind::DirectHttps,
            budget: Arc::new(Mutex::new(IpBudget::new())),
        };
        let token = sec.primary_token().unwrap();

        let auth: ServeAuth =
            serde_json::from_slice(&fs::read(dir.path().join(SERVE_AUTH_REL)).unwrap()).unwrap();
        assert!(constant_time_hash_match(&auth.token_sha256, token));
        assert!(!constant_time_hash_match(&auth.token_sha256, "bad"));

        // Valid GET & HEAD
        let ok = route_request(
            &Method::GET,
            &format!("/sub/{token}/clash.yaml"),
            None,
            &state,
            None,
        );
        assert_eq!(ok.status(), StatusCode::OK);
        assert!(ok.headers().contains_key("Profile-Title"));
        assert_eq!(
            ok.headers().get("X-Robots-Tag").unwrap(),
            "noindex, nofollow"
        );
        assert!(ok.body().size_hint().exact().unwrap() > 100);

        let head = route_request(
            &Method::HEAD,
            &format!("/sub/{token}/sing-box.json"),
            None,
            &state,
            None,
        );
        assert_eq!(head.status(), StatusCode::OK);

        // Query string rejected with 404
        let q = route_request(
            &Method::GET,
            &format!("/sub/{token}/clash.yaml"),
            Some("format=clash"),
            &state,
            None,
        );
        assert_eq!(q.status(), StatusCode::NOT_FOUND);

        // Wrong token rejected with 404
        let bad_tok = route_request(&Method::GET, "/sub/wrong/clash.yaml", None, &state, None);
        assert_eq!(bad_tok.status(), StatusCode::NOT_FOUND);

        // POST rejected with 405
        let post = route_request(
            &Method::POST,
            &format!("/sub/{token}/clash.yaml"),
            None,
            &state,
            None,
        );
        assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
