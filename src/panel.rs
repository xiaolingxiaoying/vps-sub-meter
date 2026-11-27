mod ops;

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore};
use tokio::time::timeout;
use vps_sub_meter::{config::Config, secrets::random_urlsafe, store};

const HTML: &str = include_str!("../templates/panel/index.html");
const CSS: &str = include_str!("../templates/panel/panel.css");
const JS: &str = include_str!("../templates/panel/panel.js");
const MAX_BODY_BYTES: usize = 128 * 1024;
const RECENT_JOBS: usize = 12;

struct State {
    root: PathBuf,
    key_hashes: [String; 1],
    jobs: Mutex<VecDeque<Job>>,
    close: Notify,
}

#[derive(Serialize)]
struct Job {
    id: String,
    state: JobState,
    closes_panel: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum JobState {
    Running,
    Done,
    Failed,
}

type Reply = Response<Full<Bytes>>;

pub fn run(root: &Path, port: u16) -> Result<(), String> {
    let live = root == Path::new("/");
    let root = if live {
        root.to_path_buf()
    } else {
        root.canonicalize().map_err(|error| error.to_string())?
    };
    if root == Path::new("/") {
        #[cfg(not(target_os = "linux"))]
        return Err("系统部署面板仅支持 Linux。请用 --root 指定测试目录。".into());
        #[cfg(target_os = "linux")]
        if unsafe { libc::geteuid() } != 0 {
            return Err("请使用 sudo vsm panel 启动 VPS 管理面板".into());
        }
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?
        .block_on(serve(root, port))
}

async fn serve(root: PathBuf, port: u16) -> Result<(), String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|error| format!("无法监听 127.0.0.1:{port}：{error}"))?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let key = random_urlsafe(32).map_err(|error| error.to_string())?;
    eprintln!("管理面板：http://{address}/");
    eprintln!("本次会话密钥：{key}");
    eprintln!(
        "请在本机通过 SSH 转发访问（例如 ssh -L 127.0.0.1:{}:127.0.0.1:{} ...），按 Ctrl+C 关闭面板。",
        address.port(),
        address.port()
    );
    let state = Arc::new(State {
        root,
        key_hashes: [store::hash_token(&key)],
        jobs: Mutex::new(VecDeque::new()),
        close: Notify::new(),
    });
    drop(key);
    let connections = Arc::new(Semaphore::new(24));
    loop {
        let (socket, _) = tokio::select! {
            result = listener.accept() => result.map_err(|error| error.to_string())?,
            () = state.close.notified() => return Ok(()),
        };
        let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else {
            continue;
        };
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let _permit = permit;
            let service = service_fn(move |request| {
                let state = Arc::clone(&state);
                async move { Ok::<_, std::convert::Infallible>(route(request, state).await) }
            });
            let connection = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(5))
                .max_buf_size(16 * 1024)
                .keep_alive(false)
                .serve_connection(TokioIo::new(socket), service);
            let _ = timeout(Duration::from_secs(35), connection).await;
        });
    }
}

async fn route(request: Request<Incoming>, state: Arc<State>) -> Reply {
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_loopback_authority);
    let origin_matches = match (&host, headers.get(header::ORIGIN)) {
        (Some(host), Some(origin)) => origin
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("http://"))
            .and_then(normalize_loopback_authority)
            .is_some_and(|origin_host| origin_host == *host),
        (Some(_), None) => true,
        (None, _) => false,
    };
    let same_site = headers
        .get("sec-fetch-site")
        .is_none_or(|value| matches!(value.as_bytes(), b"same-origin" | b"none"));
    if host.is_none() || !origin_matches || !same_site || request.uri().authority().is_some() {
        return error(StatusCode::FORBIDDEN, "仅允许面板自身来源访问");
    }
    if request.uri().query().is_some() {
        return error(StatusCode::NOT_FOUND, "页面不存在");
    }
    let method = request.method().clone();
    let path = request.uri().path();
    if method == Method::GET {
        match path {
            "/" => return reply(StatusCode::OK, "text/html; charset=utf-8", HTML),
            "/panel.css" => return reply(StatusCode::OK, "text/css; charset=utf-8", CSS),
            "/panel.js" => {
                return reply(StatusCode::OK, "text/javascript; charset=utf-8", JS);
            }
            "/favicon.ico" => return reply(StatusCode::NO_CONTENT, "image/x-icon", ""),
            _ => {}
        }
    }
    if !path.starts_with("/api/") {
        return error(StatusCode::NOT_FOUND, "页面不存在");
    }
    let authorized = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|candidate| {
            candidate.len() == 43 && store::constant_time_hash_match(&state.key_hashes, candidate)
        });
    if !authorized {
        return error(StatusCode::UNAUTHORIZED, "请输入 SSH 终端显示的会话密钥");
    }
    if method == Method::GET
        && let Some(id) = path.strip_prefix("/api/jobs/")
    {
        return job_status(&state, id);
    }
    if method == Method::GET && path == "/api/session" {
        return json_reply(StatusCode::OK, &json!({"authenticated": true}));
    }
    if state
        .jobs
        .lock()
        .expect("panel jobs lock")
        .iter()
        .any(|job| job.closes_panel && job.state != JobState::Failed)
    {
        return error(
            StatusCode::CONFLICT,
            "升级或卸载正在进行，完成后本次面板会话将关闭",
        );
    }
    match (method, path) {
        (Method::GET, "/api/state") => read(&state, ops::state).await,
        (Method::GET, "/api/tokens") => read(&state, ops::tokens).await,
        (Method::GET, "/api/defaults") => {
            read(&state, |root| {
                let config = Config::load_defaults(root).map_err(|error| error.to_string())?;
                serde_json::to_value(config).map_err(|error| error.to_string())
            })
            .await
        }
        (Method::POST, "/api/action") => action(request, state).await,
        _ => error(StatusCode::NOT_FOUND, "接口不存在"),
    }
}

fn normalize_loopback_authority(raw: &str) -> Option<(String, u16)> {
    let (host, port) = if let Some(rest) = raw.strip_prefix("[::1]") {
        let port = match rest {
            "" => 80,
            _ => rest.strip_prefix(':')?.parse::<u16>().ok()?,
        };
        ("[::1]".to_owned(), port)
    } else if let Some((host, port_str)) = raw.split_once(':') {
        let host = host.to_ascii_lowercase();
        if host != "127.0.0.1" && host != "localhost" {
            return None;
        }
        (host, port_str.parse::<u16>().ok()?)
    } else {
        let host = raw.to_ascii_lowercase();
        if host != "127.0.0.1" && host != "localhost" {
            return None;
        }
        (host, 80)
    };
    (port != 0).then_some((host, port))
}

async fn read(
    state: &State,
    task: impl FnOnce(&Path) -> Result<Value, String> + Send + 'static,
) -> Reply {
    let root = state.root.clone();
    match tokio::task::spawn_blocking(move || task(&root)).await {
        Ok(Ok(data)) => json_reply(StatusCode::OK, &data),
        Ok(Err(detail)) => error(StatusCode::UNPROCESSABLE_ENTITY, &detail),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "读取运行状态失败"),
    }
}

async fn action(request: Request<Incoming>, state: Arc<State>) -> Reply {
    let json_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(';').next() == Some("application/json"));
    if !json_type {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "请求必须使用 JSON");
    }
    let body = match timeout(
        Duration::from_secs(8),
        Limited::new(request.into_body(), MAX_BODY_BYTES).collect(),
    )
    .await
    {
        Ok(Ok(body)) => body.to_bytes(),
        Ok(Err(_)) => return error(StatusCode::PAYLOAD_TOO_LARGE, "请求过大或读取失败"),
        Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "读取请求超时"),
    };
    let action: ops::Action = match serde_json::from_slice(&body) {
        Ok(action) => action,
        Err(detail) => {
            return error(StatusCode::BAD_REQUEST, &format!("请求参数无效：{detail}"));
        }
    };
    if action.is_read_only() {
        return read(&state, move |root| action.execute(root)).await;
    }
    let id = match random_urlsafe(12) {
        Ok(id) => id,
        Err(detail) => return error(StatusCode::INTERNAL_SERVER_ERROR, &detail.to_string()),
    };
    let closes_panel = action.closes_panel();
    {
        let mut jobs = state.jobs.lock().expect("panel jobs lock");
        if jobs.iter().any(|job| {
            job.state == JobState::Running || (job.closes_panel && job.state == JobState::Done)
        }) {
            return error(StatusCode::CONFLICT, "已有管理操作正在执行，请等待完成");
        }
        jobs.push_back(Job {
            id: id.clone(),
            state: JobState::Running,
            closes_panel,
            result: None,
        });
        if jobs.len() > RECENT_JOBS {
            jobs.pop_front();
        }
    }
    let job_id = id.clone();
    tokio::spawn(async move {
        let root = state.root.clone();
        let task = tokio::task::spawn_blocking(move || action.execute(&root)).await;
        let (status, result) = match task {
            Ok(Ok(value)) => (JobState::Done, value),
            Ok(Err(detail)) => (JobState::Failed, json!({"message": detail})),
            Err(_) => (JobState::Failed, json!({"message": "后台管理操作异常结束"})),
        };
        if let Some(job) = state
            .jobs
            .lock()
            .expect("panel jobs lock")
            .iter_mut()
            .find(|job| job.id == job_id)
        {
            job.state = status;
            job.result = Some(result);
        }
        if closes_panel && status == JobState::Done {
            tokio::time::sleep(Duration::from_secs(45)).await;
            state.close.notify_one();
        }
    });
    json_reply(StatusCode::ACCEPTED, &json!({"id": id}))
}

fn job_status(state: &Arc<State>, id: &str) -> Reply {
    let jobs = state.jobs.lock().expect("panel jobs lock");
    let Some(job) = jobs.iter().find(|job| job.id == id) else {
        return error(StatusCode::NOT_FOUND, "找不到管理操作");
    };
    if job.state == JobState::Done && job.closes_panel {
        let state = Arc::clone(state);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            state.close.notify_one();
        });
    }
    json_reply(StatusCode::OK, job)
}

fn reply(status: StatusCode, content_type: &'static str, body: impl Into<Bytes>) -> Reply {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .header("x-frame-options", "DENY")
        .header("referrer-policy", "no-referrer")
        .header(
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src blob:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        )
        .body(Full::new(body.into()))
        .expect("static panel response headers")
}

fn json_reply(status: StatusCode, body: &impl Serialize) -> Reply {
    match serde_json::to_vec(body) {
        Ok(body) => reply(status, "application/json; charset=utf-8", body),
        Err(_) => reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "application/json; charset=utf-8",
            "{\"error\":\"响应生成失败\"}",
        ),
    }
}

fn error(status: StatusCode, detail: &str) -> Reply {
    json_reply(status, &json!({"error": detail}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_authority_accepts_localhost_and_forwarded_ports_and_rejects_foreign_hosts() {
        assert_eq!(
            normalize_loopback_authority("127.0.0.1:8787"),
            Some(("127.0.0.1".into(), 8787))
        );
        assert_eq!(
            normalize_loopback_authority("localhost:9999"),
            Some(("localhost".into(), 9999))
        );
        assert_eq!(
            normalize_loopback_authority("LOCALHOST"),
            Some(("localhost".into(), 80))
        );
        assert_eq!(
            normalize_loopback_authority("[::1]:8787"),
            Some(("[::1]".into(), 8787))
        );
        assert_eq!(
            normalize_loopback_authority("[::1]"),
            Some(("[::1]".into(), 80))
        );
        assert_eq!(normalize_loopback_authority("127.0.0.1:0"), None);
        assert_eq!(normalize_loopback_authority("evil.example:8787"), None);
        assert_eq!(normalize_loopback_authority("127.0.0.1:8787/path"), None);
        assert_eq!(normalize_loopback_authority("127.0.0.1:8787:80"), None);
    }
}
