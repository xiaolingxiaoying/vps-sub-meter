use std::io::{self, Write};
use std::path::Path;

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::json;
use vps_sub_meter::{
    apply, compiler,
    config::{Config, ConfigError, VAR_DIR, randomize_node_ports},
    fs_util, qr,
    secrets::{Secrets, SubscriptionToken, random_urlsafe},
};

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// 输出当前配置或默认模板的 JSON
    Show {
        #[arg(long)]
        defaults: bool,
    },
    /// 生成并校验配置预览（JSON）
    Prepare(PrepareArgs),
    /// 从标准输入读取候选配置 JSON，并保存为 TOML
    Write {
        #[arg(long)]
        replace: bool,
    },
}

#[derive(Args)]
pub struct PrepareArgs {
    #[arg(long)]
    domain: Option<String>,
    #[arg(long)]
    public_ip: Option<String>,
    #[arg(long)]
    interface: Option<String>,
    #[arg(long)]
    monthly_gib: Option<f64>,
    #[arg(long)]
    reset_day: Option<u8>,
    #[arg(long)]
    ports: Option<Ports>,
}

#[derive(Clone, ValueEnum)]
enum Ports {
    Keep,
    Random,
}

#[derive(Subcommand)]
pub enum TokenCmd {
    /// 列出脱敏后的订阅凭据
    List {
        #[arg(long)]
        json: bool,
    },
    /// 新增一个设备凭据
    Add { name: String },
    /// 轮换默认订阅凭据
    Rotate,
    /// 吊销指定名称的订阅凭据
    Revoke { name: String },
}

#[derive(Args)]
pub struct SubArgs {
    #[arg(long, conflicts_with_all = ["qr", "qr_format"])]
    json: bool,
    #[arg(long)]
    qr: bool,
    #[arg(long, value_enum)]
    qr_format: Option<QrFormat>,
    #[arg(long)]
    token_name: Option<String>,
}

#[derive(Clone, ValueEnum)]
enum QrFormat {
    Clash,
    SingBox,
    Shadowrocket,
}

pub fn config(root: &Path, command: ConfigCmd) -> Result<(), String> {
    match command {
        ConfigCmd::Show { defaults } => {
            let config = if defaults {
                Config::load_defaults(root)
            } else {
                Config::load(root)
            }
            .map_err(|e| e.to_string())?;
            print_json(&config)
        }
        ConfigCmd::Prepare(args) => {
            let (mut config, fresh) = match Config::load(root) {
                Ok(config) => (config, false),
                Err(ConfigError::Missing(_)) => (
                    Config::load_defaults(root).map_err(|e| e.to_string())?,
                    true,
                ),
                Err(error) => return Err(error.to_string()),
            };
            config.domain = args.domain.unwrap_or(config.domain);
            config.public_ip = args.public_ip.unwrap_or(config.public_ip);
            config.interface = args.interface.unwrap_or(config.interface);
            config.traffic.monthly_gb = args.monthly_gib.unwrap_or(config.traffic.monthly_gb);
            config.traffic.reset_day = args.reset_day.unwrap_or(config.traffic.reset_day);
            if matches!(
                args.ports
                    .unwrap_or(if fresh { Ports::Random } else { Ports::Keep }),
                Ports::Random
            ) {
                randomize_node_ports(&mut config, root == Path::new("/"))?;
            }
            config.validate().map_err(|e| e.to_string())?;
            let mode = if config.is_ip_mode() {
                "ip"
            } else if config.xray_fronts_443() {
                "https-shared"
            } else {
                "https"
            };
            print_json(&json!({"config": config, "mode": mode, "tls_sni": config.tls_sni()}))
        }
        ConfigCmd::Write { replace } => {
            let config: Config = serde_json::from_reader(io::stdin().lock())
                .map_err(|e| format!("配置 JSON 无效: {e}"))?;
            config.validate().map_err(|e| e.to_string())?;
            let _lock = fs_util::lock_exclusive(&root.join(VAR_DIR)).map_err(|e| e.to_string())?;
            if !replace {
                match Config::path(root).symlink_metadata() {
                    Ok(_) => return Err("主配置已存在，覆盖时需要 --replace".into()),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            config.save(root).map_err(|e| e.to_string())
        }
    }
}

pub fn sub(root: &Path, args: SubArgs) -> Result<(), String> {
    let config = Config::load(root).map_err(|e| e.to_string())?;
    let secrets = Secrets::load_or_default(root).map_err(|e| e.to_string())?;
    let token = match args.token_name {
        Some(name) => &secrets.tokens[token_index(&secrets.tokens, &name)?].token,
        None => secrets.primary_token().ok_or("尚未生成订阅凭据")?,
    };
    let base = config.subscription_base_url(token);
    if args.json {
        let argo = config.argo.enabled.then(|| {
            subscription_links(&format!(
                "https://{}/sub/{token}",
                config.argo.sub_domain.trim()
            ))
        });
        return print_json(&json!({"direct": subscription_links(&base), "argo": argo}));
    }

    println!("看板:         {base}/index");
    println!("Clash/Mihomo: {base}/clash.yaml");
    println!("sing-box:     {base}/sing-box.json");
    println!("Shadowrocket: {base}/shadowrocket.txt");
    println!("URI:          {base}/uri.txt");
    if let Some(format) = args.qr_format.or(args.qr.then_some(QrFormat::Clash)) {
        let payload = match format {
            QrFormat::Clash => format!("{base}/clash.yaml"),
            QrFormat::SingBox => format!(
                "sing-box://import-remote-profile?url={}#{}",
                compiler::pct(&format!("{base}/sing-box.json")),
                compiler::pct(&config.profile_name)
            ),
            QrFormat::Shadowrocket => format!("{base}/shadowrocket.txt"),
        };
        print!("{}", qr::render_ansi(&payload)?);
    }
    Ok(())
}

fn subscription_links(base: &str) -> serde_json::Value {
    json!({
        "index": format!("{base}/index"),
        "clash": format!("{base}/clash.yaml"),
        "sing_box": format!("{base}/sing-box.json"),
        "shadowrocket": format!("{base}/shadowrocket.txt"),
        "uri": format!("{base}/uri.txt"),
    })
}

pub fn token(root: &Path, command: TokenCmd) -> Result<(), String> {
    if let TokenCmd::List { json } = command {
        let secrets = Secrets::load_or_default(root).map_err(|e| e.to_string())?;
        let entries: Vec<_> = secrets
            .tokens
            .iter()
            .enumerate()
            .map(|(index, token)| TokenSummary {
                name: &token.name,
                created_at: token.created_at,
                masked: mask_token(&token.token),
                default: index == 0,
            })
            .collect();
        if json {
            return print_json(&entries);
        }
        for entry in entries {
            let suffix = if entry.default { "（默认）" } else { "" };
            println!("{} {}{suffix}", entry.name, entry.masked);
        }
        return Ok(());
    }

    apply::update_tokens(root, |tokens| {
        match command {
            TokenCmd::Add { name } => {
                let name = name.trim();
                if name.is_empty() || name.chars().any(char::is_control) {
                    return Err("凭据名称无效".into());
                }
                if tokens.iter().any(|token| token.name == name) {
                    return Err("凭据名称已存在".into());
                }
                tokens.push(SubscriptionToken {
                    name: name.to_owned(),
                    token: random_urlsafe(32).map_err(|e| e.to_string())?,
                    created_at: chrono::Utc::now().timestamp(),
                });
            }
            TokenCmd::Rotate => {
                let first = tokens.first_mut().ok_or("尚未生成默认凭据")?;
                first.token = random_urlsafe(32).map_err(|e| e.to_string())?;
                first.created_at = chrono::Utc::now().timestamp();
            }
            TokenCmd::Revoke { name } => {
                let index = token_index(tokens, &name)?;
                if tokens.len() == 1 {
                    return Err("不能吊销最后一个凭据，请使用轮换功能".into());
                }
                tokens.remove(index);
            }
            TokenCmd::List { .. } => unreachable!(),
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    println!("订阅凭据已更新");
    Ok(())
}

fn token_index(tokens: &[SubscriptionToken], name: &str) -> Result<usize, String> {
    let mut matches = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.name == name);
    let (index, _) = matches.next().ok_or("未找到该名称的凭据")?;
    if matches.next().is_some() {
        return Err("该名称对应多个凭据，操作已取消".into());
    }
    Ok(index)
}

#[derive(Serialize)]
struct TokenSummary<'a> {
    name: &'a str,
    created_at: i64,
    masked: String,
    default: bool,
}

fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 8 {
        return "***".into();
    }
    let first: String = chars[..4].iter().collect();
    let last: String = chars[chars.len() - 4..].iter().collect();
    format!("{first}...{last}")
}

pub fn print_json(value: &impl Serialize) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value).map_err(|e| e.to_string())?;
    writeln!(stdout).map_err(|e| e.to_string())
}
