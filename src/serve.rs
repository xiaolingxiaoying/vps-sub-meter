//! Hardened read-only HTTP/HTTPS subscription & ACME server with per-IP concurrency
//! isolation, token-bucket rate limiting, zero-downtime certificate hot-reload,
//! dark Chinese overview dashboard (`/index`), and precompiled Argo loopback mode.

use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rustls_pemfile::{certs, private_key};
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

use crate::config::{Config, pct};
use crate::qr;
use crate::store::{self, GenerationMeta, SERVE_AUTH_REL, ServeAuth};
use crate::traffic::{self, TrafficSnapshot};

const INDEX_HTML_TEMPLATE: &str = include_str!("../templates/index.html");

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

type FileStamp = (SystemTime, u64);

struct FileCache<T> {
    entry: Mutex<Option<(FileStamp, Arc<T>)>>,
}

impl<T> FileCache<T> {
    fn new() -> Self {
        Self {
            entry: Mutex::new(None),
        }
    }

    fn get_or_load<E>(&self, path: &Path, load: impl FnOnce() -> Result<T, E>) -> Option<Arc<T>> {
        let meta = fs::symlink_metadata(path).ok()?;
        let modified = meta.modified().ok()?;
        let stamp = (modified, meta.len());
        {
            let guard = self.entry.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((cached_stamp, cached)) = guard.as_ref()
                && *cached_stamp == stamp
            {
                return Some(Arc::clone(cached));
            }
        }
        let fresh = Arc::new(load().ok()?);
        let mut guard = self.entry.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some((stamp, Arc::clone(&fresh)));
        Some(fresh)
    }
}

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    kind: ListenerKind,
    budget: Arc<Mutex<IpBudget>>,
    config_cache: Arc<FileCache<Config>>,
    auth_cache: Arc<FileCache<ServeAuth>>,
}

impl AppState {
    fn new(root: PathBuf, kind: ListenerKind, budget: Arc<Mutex<IpBudget>>) -> Self {
        Self {
            root,
            kind,
            budget,
            config_cache: Arc::new(FileCache::new()),
            auth_cache: Arc::new(FileCache::new()),
        }
    }
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

#[derive(Clone)]
enum LoopMode {
    Acme {
        root: PathBuf,
    },
    Http {
        state: AppState,
    },
    Https {
        state: AppState,
        tls_rx: watch::Receiver<Option<Arc<rustls::ServerConfig>>>,
    },
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
        let state = AppState::new(
            root.clone(),
            ListenerKind::FallbackHttp,
            Arc::clone(&budget),
        );
        handles.push(tokio::spawn(accept_loop(
            listener,
            LoopMode::Http { state },
            true,
        )));
    } else {
        // ACME HTTP-01 listener
        let acme_addr: SocketAddr = ([0, 0, 0, 0], config.server.acme_port).into();
        let acme_listener = TcpListener::bind(acme_addr).await?;
        eprintln!("vps-sub-meter listening ACME HTTP on {acme_addr}");
        handles.push(tokio::spawn(accept_loop(
            acme_listener,
            LoopMode::Acme { root: root.clone() },
            true,
        )));

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
        let primary_state =
            AppState::new(root.clone(), ListenerKind::DirectHttps, Arc::clone(&budget));
        handles.push(tokio::spawn(accept_loop(
            primary_listener,
            LoopMode::Https {
                state: primary_state,
                tls_rx: tls_rx.clone(),
            },
            !config.xray_fronts_443(),
        )));

        // Optional extra HTTPS port (e.g. 8443 compatibility during migration)
        if let Some(extra) = config.server.extra_https_port {
            let extra_addr: SocketAddr = ([0, 0, 0, 0], extra).into();
            let extra_listener = TcpListener::bind(extra_addr).await?;
            eprintln!("vps-sub-meter listening extra HTTPS on {extra_addr}");
            let extra_state =
                AppState::new(root.clone(), ListenerKind::DirectHttps, Arc::clone(&budget));
            handles.push(tokio::spawn(accept_loop(
                extra_listener,
                LoopMode::Https {
                    state: extra_state,
                    tls_rx,
                },
                true,
            )));
        }
    }

    // Native Cloudflare Argo loopback HTTP listener
    if config.argo.enabled {
        let argo_addr: SocketAddr = ([127, 0, 0, 1], config.argo.loopback_sub_port).into();
        let argo_listener = TcpListener::bind(argo_addr).await?;
        eprintln!("vps-sub-meter listening Argo loopback HTTP on {argo_addr}");
        let argo_state = AppState::new(root, ListenerKind::ArgoLoopback, budget);
        handles.push(tokio::spawn(accept_loop(
            argo_listener,
            LoopMode::Http { state: argo_state },
            false,
        )));
    }

    for h in handles {
        let _ = h.await;
    }
    Ok(())
}

async fn accept_loop(listener: TcpListener, mode: LoopMode, per_peer: bool) {
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
        let m = mode.clone();
        tokio::spawn(async move {
            let _p = permit;
            match m {
                LoopMode::Acme { root } => {
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let resp = with_len(handle_acme(&req, &root));
                        async { Ok::<_, std::convert::Infallible>(resp) }
                    });
                    let _ = tokio::time::timeout(
                        CONNECTION_TIMEOUT,
                        http1_builder().serve_connection(TokioIo::new(stream), svc),
                    )
                    .await;
                }
                LoopMode::Http { state } => {
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let resp = with_len(route_request(
                            req.method(),
                            req.uri().path(),
                            req.uri().query(),
                            &state,
                            peer,
                        ));
                        async { Ok::<_, std::convert::Infallible>(resp) }
                    });
                    let _ = tokio::time::timeout(
                        CONNECTION_TIMEOUT,
                        http1_builder().serve_connection(TokioIo::new(stream), svc),
                    )
                    .await;
                }
                LoopMode::Https { state, tls_rx } => {
                    let Some(tls_cfg) = tls_rx.borrow().clone() else {
                        return;
                    };
                    let acceptor = TlsAcceptor::from(tls_cfg);
                    let Ok(Ok(tls_stream)) =
                        tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await
                    else {
                        return;
                    };
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let resp = with_len(route_request(
                            req.method(),
                            req.uri().path(),
                            req.uri().query(),
                            &state,
                            peer,
                        ));
                        async { Ok::<_, std::convert::Infallible>(resp) }
                    });
                    let _ = tokio::time::timeout(
                        CONNECTION_TIMEOUT,
                        http1_builder().serve_connection(TokioIo::new(Box::pin(tls_stream)), svc),
                    )
                    .await;
                }
            }
        });
    }
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

fn with_len(mut resp: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    use hyper::body::Body;
    let len = resp.body().size_hint().exact().unwrap_or(0);
    resp.headers_mut()
        .insert(hyper::header::CONTENT_LENGTH, len.into());
    resp
}

fn handle_acme(req: &Request<Incoming>, root: &Path) -> Response<Full<Bytes>> {
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
    match store::read_no_follow(&path) {
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

    let Some(config) = state
        .config_cache
        .get_or_load(&Config::path(&state.root), || Config::load(&state.root))
    else {
        return plain_status(StatusCode::SERVICE_UNAVAILABLE, b"Service Unavailable\n");
    };
    let Some(auth) = state
        .auth_cache
        .get_or_load(&state.root.join(SERVE_AUTH_REL), || {
            store::read_serve_auth(&state.root)
        })
    else {
        return plain_status(StatusCode::SERVICE_UNAVAILABLE, b"Service Unavailable\n");
    };

    if !store::constant_time_hash_match(&auth.token_sha256, token) {
        return plain_status(StatusCode::NOT_FOUND, b"Not Found\n");
    }

    let Ok(Some(gen_meta)) = store::current_generation(&state.root) else {
        return plain_status(StatusCode::SERVICE_UNAVAILABLE, b"Not Ready\n");
    };

    let is_argo = state.kind == ListenerKind::ArgoLoopback;
    if is_argo && (!config.argo.enabled || gen_meta.argo_files.is_empty()) {
        return plain_status(StatusCode::NOT_FOUND, b"Argo Not Enabled\n");
    }

    let snap = traffic::snapshot(&state.root, &config).ok();
    let userinfo = snap.as_ref().map(|s| s.userinfo_header());
    let (base_url, profile_title) = if is_argo {
        (
            format!("https://{}/sub/{token}", config.argo.sub_domain.trim()),
            config.argo.profile_name.as_str(),
        )
    } else {
        (
            config.subscription_base_url(token),
            config.profile_name.as_str(),
        )
    };

    serve_route(
        &state.root,
        &tail,
        &config,
        &gen_meta,
        &base_url,
        profile_title,
        snap.as_ref(),
        userinfo.as_deref(),
        is_argo,
    )
}

#[allow(clippy::too_many_arguments)]
fn serve_route(
    root: &Path,
    tail: &str,
    config: &Config,
    gen_meta: &GenerationMeta,
    base_url: &str,
    profile_title: &str,
    snap: Option<&TrafficSnapshot>,
    userinfo: Option<&str>,
    is_argo: bool,
) -> Response<Full<Bytes>> {
    let title_b64 = format!("base64:{}", STANDARD.encode(profile_title.as_bytes()));
    let index_url = format!("{base_url}/index");

    let has_singbox = if is_argo {
        gen_meta.argo_files.iter().any(|f| f == "sing-box.json")
    } else {
        !gen_meta.singbox_nodes.is_empty()
    };

    match tail {
        "clash.yaml" => serve_artifact(
            root,
            &gen_meta.id,
            is_argo,
            "clash.yaml",
            "text/yaml; charset=utf-8",
            Some("vsm-clash.yaml"),
            Some(&title_b64),
            Some(&index_url),
            userinfo,
        ),
        "sing-box.json" => {
            if !has_singbox {
                return plain_status(StatusCode::NOT_FOUND, b"No compatible sing-box nodes\n");
            }
            serve_artifact(
                root,
                &gen_meta.id,
                is_argo,
                "sing-box.json",
                "application/json; charset=utf-8",
                Some("sing-box.json"),
                Some(&title_b64),
                Some(&index_url),
                userinfo,
            )
        }
        "shadowrocket.txt" => serve_artifact(
            root,
            &gen_meta.id,
            is_argo,
            "shadowrocket.txt",
            "text/plain; charset=utf-8",
            Some("shadowrocket.txt"),
            Some(&title_b64),
            Some(&index_url),
            userinfo,
        ),
        "uri.txt" => serve_artifact(
            root,
            &gen_meta.id,
            is_argo,
            "uri.txt",
            "text/plain; charset=utf-8",
            None,
            None,
            None,
            userinfo,
        ),
        "" | "index" => {
            let uri_txt = store::read_generation_file(root, &gen_meta.id, is_argo, "uri.txt")
                .ok()
                .flatten()
                .unwrap_or_default();
            let html = build_index_html(
                config,
                gen_meta,
                &uri_txt,
                base_url,
                profile_title,
                snap,
                has_singbox,
                is_argo,
            );
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
        "qr/clash.svg" => render_qr_response(&format!("{base_url}/clash.yaml"), is_argo),
        "qr/sing-box.svg" => {
            if !has_singbox {
                return plain_status(StatusCode::NOT_FOUND, b"Not Found\n");
            }
            let payload = format!(
                "sing-box://import-remote-profile?url={}#{}",
                pct(&format!("{base_url}/sing-box.json")),
                pct(profile_title)
            );
            render_qr_response(&payload, is_argo)
        }
        "qr/shadowrocket.svg" => {
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
        "qr/uri.svg" => render_qr_response(&format!("{base_url}/uri.txt"), is_argo),
        _ => plain_status(StatusCode::NOT_FOUND, b"Not Found\n"),
    }
}

#[allow(clippy::too_many_arguments)]
fn serve_artifact(
    root: &Path,
    gen_id: &str,
    is_argo: bool,
    file: &str,
    content_type: &str,
    filename: Option<&str>,
    profile_title: Option<&str>,
    web_url: Option<&str>,
    userinfo: Option<&str>,
) -> Response<Full<Bytes>> {
    match store::read_generation_file(root, gen_id, is_argo, file) {
        Ok(Some(text)) => build_sub_resp(
            StatusCode::OK,
            content_type,
            text.as_bytes(),
            filename,
            profile_title,
            web_url,
            userinfo,
            is_argo,
        ),
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

#[allow(clippy::too_many_arguments)]
fn build_index_html(
    config: &Config,
    gen_meta: &GenerationMeta,
    uri_txt: &str,
    base_url: &str,
    title: &str,
    snap: Option<&TrafficSnapshot>,
    has_singbox: bool,
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
    let node_names = if is_argo {
        &gen_meta.argo_nodes
    } else {
        &gen_meta.direct_nodes
    };
    if has_singbox {
        let sb_url = format!("{base_url}/sing-box.json");
        let sb_qr = format!(
            "sing-box://import-remote-profile?url={}#{}",
            pct(&sb_url),
            pct(title)
        );
        let sb_count = if is_argo {
            node_names.len()
        } else {
            gen_meta.singbox_nodes.len()
        };
        push_card(
            &format!("sing-box 1.14+ 订阅 ({sb_count} 节点)"),
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

    let nodes_summary = esc(&node_names.join(" · "));
    let uri_escaped = esc(uri_txt.trim());

    INDEX_HTML_TEMPLATE
        .replace("{{TITLE}}", &esc(title))
        .replace("{{NODES_SUMMARY}}", &nodes_summary)
        .replace("{{USED}}", &used_str)
        .replace("{{TOTAL}}", &total_str)
        .replace("{{PCT}}", &pct_val)
        .replace("{{UPLOAD}}", &up_str)
        .replace("{{DOWNLOAD}}", &down_str)
        .replace("{{RESET}}", &reset_str)
        .replace("{{NODE_COUNT}}", &node_names.len().to_string())
        .replace("{{CARDS}}", &cards)
        .replace("{{URI_ESCAPED}}", &uri_escaped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply;
    use hyper::body::Body;

    #[test]
    fn rejects_invalid_tokens_and_query_params_and_serves_valid_and_argo_routes() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        cfg.argo.enabled = true;
        cfg.argo.tunnel_domain = "tunnel.example.com".into();
        cfg.argo.sub_domain = "argo.example.com".into();
        cfg.save(dir.path()).unwrap();
        let sec = crate::test_secrets::fixture(&cfg);
        sec.save(dir.path()).unwrap();
        apply::apply_all(dir.path()).unwrap();

        let budget = Arc::new(Mutex::new(IpBudget::new()));
        let direct_state = AppState::new(
            dir.path().to_path_buf(),
            ListenerKind::DirectHttps,
            Arc::clone(&budget),
        );
        let argo_state =
            AppState::new(dir.path().to_path_buf(), ListenerKind::ArgoLoopback, budget);
        let token = sec.primary_token().unwrap();

        let auth = store::read_serve_auth(dir.path()).unwrap();
        assert!(store::constant_time_hash_match(&auth.token_sha256, token));
        assert!(!store::constant_time_hash_match(&auth.token_sha256, "bad"));

        // Valid GET & HEAD
        let ok = route_request(
            &Method::GET,
            &format!("/sub/{token}/clash.yaml"),
            None,
            &direct_state,
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
            &direct_state,
            None,
        );
        assert_eq!(head.status(), StatusCode::OK);

        // Index & QR SVG
        let idx = route_request(
            &Method::GET,
            &format!("/sub/{token}/index"),
            None,
            &direct_state,
            None,
        );
        assert_eq!(idx.status(), StatusCode::OK);
        let qr_svg = route_request(
            &Method::GET,
            &format!("/sub/{token}/qr/clash.svg"),
            None,
            &direct_state,
            None,
        );
        assert_eq!(qr_svg.status(), StatusCode::OK);

        // Precompiled Argo serving
        let argo_ok = route_request(
            &Method::GET,
            &format!("/sub/{token}/clash.yaml"),
            None,
            &argo_state,
            None,
        );
        assert_eq!(argo_ok.status(), StatusCode::OK);
        assert_eq!(
            argo_ok.headers().get("CDN-Cache-Control").unwrap(),
            "no-store"
        );

        // Legacy alias removed
        let legacy = route_request(
            &Method::GET,
            &format!("/sub/{token}/sing-box-full.json"),
            None,
            &direct_state,
            None,
        );
        assert_eq!(legacy.status(), StatusCode::NOT_FOUND);

        // Query string rejected with 404
        let q = route_request(
            &Method::GET,
            &format!("/sub/{token}/clash.yaml"),
            Some("format=clash"),
            &direct_state,
            None,
        );
        assert_eq!(q.status(), StatusCode::NOT_FOUND);

        // Wrong token rejected with 404
        let bad_tok = route_request(
            &Method::GET,
            "/sub/wrong/clash.yaml",
            None,
            &direct_state,
            None,
        );
        assert_eq!(bad_tok.status(), StatusCode::NOT_FOUND);

        // POST rejected with 405
        let post = route_request(
            &Method::POST,
            &format!("/sub/{token}/clash.yaml"),
            None,
            &direct_state,
            None,
        );
        assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
