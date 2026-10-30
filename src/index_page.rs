//! The self-contained Chinese overview page served at `/sub/<credential>/index`.
//!
//! The page opens with a per-client quick-pick table (Clash Party, Clash Verge,
//! sing-box, V2rayN, Shadowrocket), then lists every subscription link with its
//! version label, the matching scannable QR code, and per-client import
//! instructions. Native proxy share links appear in a collapsed disclosure so
//! credentials are not immediately visible when the overview opens. It carries
//! no external resources, so it renders offline and a credential rotation
//! invalidates the whole page together with every link it shows. A broken
//! accounting state degrades the traffic badges but never the links themselves.

use base64::Engine;
use chrono_tz::Tz;

use crate::config::{DeploymentConfig, DeploymentStore};
use crate::subscription::{
    ClientVersion, SubscriptionFormat, SubscriptionRoute, available_client_subscription_matrix,
    available_subscription_matrix,
};

/// Renders the overview page. Fails only when a QR code or a route URL cannot
/// be produced; the caller turns any failure into a redacted 503. Traffic
/// reporting failures degrade to a "暂不可用" badge instead.
pub fn render(store: &DeploymentStore, config: &DeploymentConfig) -> Result<String, String> {
    render_with_credential(store, config, &config.subscription_credential)
}

/// Render links using the credential that authorized this request.
pub fn render_with_credential(
    store: &DeploymentStore,
    config: &DeploymentConfig,
    credential: &str,
) -> Result<String, String> {
    let traffic = match crate::traffic::report(store, config) {
        Ok(traffic) => Some(traffic),
        Err(error) => {
            // The page degrades to a "暂不可用" badge, but the reason still has
            // to reach the journal or an administrator cannot diagnose it.
            eprintln!(
                "index page traffic unavailable: {}",
                crate::subscription::redact_secret(
                    &error.to_string(),
                    &config.subscription_credential
                )
            );
            None
        }
    };
    let mut client_rows = String::new();
    for row in available_client_subscription_matrix(config) {
        client_rows.push_str("<tr><th>");
        client_rows.push_str(&esc(row.client));
        client_rows.push_str("</th><td><ul>");
        for (index, recommended) in row.formats.iter().enumerate() {
            let url = crate::subscription::route_url_with_credential(
                config,
                credential,
                SubscriptionRoute::Format(recommended.format),
            )
            .map_err(|error| error.to_string())?;
            let primary = index == 0;
            client_rows.push_str(&format!(
                "<li><a href=\"{url_attr}\" rel=\"noreferrer\">{url}</a> \
                 <button type=\"button\" class=\"btn btn-sm\" onclick=\"copyUrl(this, &#39;{url_attr}&#39;)\">复制</button>\
                 <span class=\"note\">{note}{primary}</span></li>",
                url_attr = esc_attr(&url),
                url = esc(&url),
                note = esc(&recommended.note),
                primary = if primary { "；首选" } else { "" },
            ));
        }
        client_rows.push_str(&format!(
            "</ul><p class=\"note\">{note}</p></td></tr>",
            note = esc(&row.note),
        ));
    }
    // A named credential must not point the operator at `rotate` without
    // `--name`; that command only rotates the deployment default and leaves the
    // leaked device link valid. Quote the name for a shell if it contains one.
    let rotate_command = config
        .subscription_credentials
        .iter()
        .find(|entry| {
            entry.credential == credential && entry.is_active(chrono::Utc::now().timestamp())
        })
        .map(|entry| {
            format!(
                "sbctl credential rotate --name {}",
                shell_quote(&entry.name)
            )
        })
        .unwrap_or_else(|| "sbctl credential rotate".to_owned());
    let mut node_rows = String::new();
    for node in crate::canonical::nodes(config) {
        node_rows.push_str("<tr><th>");
        node_rows.push_str(&esc(node.protocol().label_zh()));
        node_rows.push_str(&format!(
            "</th><td><code>{}</code></td></tr>",
            esc(crate::subscription::node_share_link(config, &node).trim())
        ));
    }
    let mut rows = String::new();
    for info in available_subscription_matrix(config) {
        let url = crate::subscription::route_url_with_credential(
            config,
            credential,
            SubscriptionRoute::Format(info.format),
        )
        .map_err(|error| error.to_string())?;
        let qr_url = crate::subscription::route_url_with_credential(
            config,
            credential,
            SubscriptionRoute::Qr(info.format),
        )
        .map_err(|error| error.to_string())?;
        let qr_payload = info.format.qr_payload(&url);
        let qr = crate::qr::render_svg(&qr_payload)?;

        let mut import_btn = String::new();
        match info.format {
            SubscriptionFormat::Clash | SubscriptionFormat::ClashLegacy(_) => {
                let encoded: String =
                    url::form_urlencoded::byte_serialize(url.as_bytes()).collect();
                import_btn.push_str(&format!(
                    "<a class=\"btn\" href=\"clash://install-config?url={encoded}\" rel=\"noreferrer\">一键导入 Clash</a>"
                ));
            }
            SubscriptionFormat::SingBox
            | SubscriptionFormat::SingBoxFull
            | SubscriptionFormat::SingBoxVersion(_) => {
                let encoded: String =
                    url::form_urlencoded::byte_serialize(url.as_bytes()).collect();
                import_btn.push_str(&format!(
                    "<a class=\"btn\" href=\"sing-box://import-remote-profile?url={encoded}\" rel=\"noreferrer\">一键导入 sing-box</a>"
                ));
            }
            SubscriptionFormat::Shadowrocket
            | SubscriptionFormat::Uri
            | SubscriptionFormat::Base64Uri => {
                let b64 = base64::prelude::BASE64_STANDARD.encode(url.as_bytes());
                import_btn.push_str(&format!(
                    "<a class=\"btn\" href=\"sub://{b64}\" rel=\"noreferrer\">一键导入 Shadowrocket</a>"
                ));
            }
        }

        rows.push_str(&format!(
            "<details open><summary><span class=\"label\">{label}</span>\
<span class=\"audience\">{audience}</span></summary>\
<p class=\"note\">{note}</p>\
<p class=\"url\"><a href=\"{url_attr}\" rel=\"noreferrer\">{url}</a></p>\
<div class=\"actions\">\
<button type=\"button\" class=\"btn btn-primary\" onclick=\"copyUrl(this, &#39;{url_attr}&#39;)\">复制链接</button>\
{import_btn}\
<a class=\"btn\" href=\"{qr_url_attr}\" target=\"_blank\" rel=\"noreferrer\">独立二维码</a>\
</div>\
<div class=\"qr qr-card\">\
<a href=\"{qr_url_attr}\" target=\"_blank\" rel=\"noreferrer\" title=\"在新标签页放大查看独立二维码\">{qr}</a>\
<span class=\"qr-tip\">手机扫码或点击放大</span>\
</div></details>",
            label = esc(&info.label),
            audience = esc(&info.audience),
            note = esc(&info.note),
            url = esc(&url),
            url_attr = esc_attr(&url),
            qr_url_attr = esc_attr(&qr_url),
            import_btn = import_btn,
            qr = qr,
        ));
    }
    Ok(format!(
        "<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<meta name=\"robots\" content=\"noindex, nofollow\">\n\
<title>sbctl 订阅中心</title>\n\
<style>\n\
:root {{ color-scheme: light dark; }}\n\
body {{ font-family: system-ui, -apple-system, sans-serif; max-width: 48rem; margin: 0 auto; padding: 1rem 1rem 3rem; line-height: 1.5; }}\n\
h1 {{ font-size: 1.3rem; }}\n\
h2 {{ font-size: 1.1rem; margin-top: 1.6rem; }}\n\
.badge {{ display: inline-block; border: 1px solid #8884; border-radius: .4rem; padding: 0 .5rem; margin: .1rem .2rem .6rem 0; font-size: .85rem; }}\n\
details {{ border: 1px solid #8884; border-radius: .5rem; padding: .4rem .8rem; margin: .5rem 0; }}\n\
summary {{ cursor: pointer; font-weight: 600; display: flex; align-items: center; gap: .5rem; flex-wrap: wrap; }}\n\
.label {{ font-size: 1rem; }}\n\
.audience {{ font-size: .8rem; color: #888; font-weight: 400; }}\n\
.note {{ color: #888; font-size: .85rem; margin: .4rem 0 0; }}\n\
.url {{ font-size: .8rem; word-break: break-all; margin: .3rem 0; }}\n\
.actions {{ display: flex; gap: .5rem; flex-wrap: wrap; align-items: center; margin: .5rem 0; }}\n\
.btn {{ display: inline-flex; align-items: center; padding: .35rem .75rem; font-size: .82rem; font-weight: 500; border-radius: .35rem; border: 1px solid #8886; background: #8881; color: inherit; text-decoration: none; cursor: pointer; user-select: none; transition: background .15s ease; }}\n\
.btn:hover {{ background: #8883; text-decoration: none; }}\n\
.btn-primary {{ background: #1976d2; color: #ffffff !important; border-color: #1976d2; }}\n\
.btn-primary:hover {{ background: #1565c0; border-color: #1565c0; }}\n\
.btn-success {{ background: #2e7d32 !important; color: #ffffff !important; border-color: #2e7d32 !important; }}\n\
.btn-sm {{ padding: .15rem .45rem; font-size: .75rem; margin-left: .35rem; vertical-align: middle; }}\n\
.qr, .qr-card {{ display: inline-block; background: #ffffff !important; color: #1a1a1a; padding: 16px; border-radius: 12px; border: 1px solid #8884; box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12); margin: .6rem 0; max-width: 280px; width: 100%; box-sizing: border-box; text-align: center; }}\n\
.qr-card a {{ display: block; text-decoration: none; }}\n\
.qr svg, .qr-card svg {{ display: block; width: 100%; height: auto; aspect-ratio: 1 / 1; shape-rendering: crispEdges; }}\n\
.qr-tip {{ display: block; font-size: .75rem; color: #666; margin-top: .5rem; user-select: none; }}\n\
table {{ border-collapse: collapse; width: 100%; font-size: .85rem; }}\n\
th, td {{ border: 1px solid #8884; padding: .4rem .6rem; text-align: left; vertical-align: top; }}\n\
th {{ white-space: nowrap; }}\n\
td ul {{ margin: 0; padding-left: 1rem; }}\n\
td li {{ margin: .2rem 0; word-break: break-all; }}\n\
code {{ background: #8882; border-radius: .3rem; padding: 0 .3rem; }}\n\
</style>\n</head>\n<body>\n\
<h1>sbctl 订阅中心</h1>\n\
<p><span class=\"badge\">主机：{host}</span>\
<span class=\"badge\">模式：{mode}</span>\
{traffic_badges}</p>\n\
<p class=\"note\">二维码已适配各客户端扫码标准（sing-box 扫码直接识别导入；Clash/Shadowrocket 扫码获取订阅）。也可点击对应按钮「一键导入」或「复制链接」。订阅凭据泄露时请在服务器上执行 <code>{rotate_command}</code> 更换。</p>\n\
<h2>按客户端选择</h2>\n\
<table>\n{client_rows}</table>\n\
<h2>节点与原生分享链接</h2>\n\
<details><summary>查看节点分享链接（含节点凭据）</summary>\n\
<p class=\"note\">下面每条链接与 <code>uri</code> 订阅工件内容一致，含节点凭据；本页本身已受订阅凭据保护，请勿将其截图或转发。</p>\n\
<table>\n{node_rows}</table>\n\
</details>\n\
<h2>全部订阅链接</h2>\n\
{rows}\n\
<h2>客户端导入</h2>\n\
<ul>\n\
<li><strong>Clash Party / Clash Verge</strong>：粘贴 <code>clash.yaml</code> 链接导入订阅；1.18 内核使用 <code>clash-1.18.yaml</code>，不含 AnyTLS 节点；仅启用 AnyTLS 时无此旧版订阅。</li>\n\
<li><strong>sing-box（SFA / SFI / SFW / 桌面版）</strong>：先查看客户端内核版本，再选择对应 <code>sing-box-&lt;版本&gt;.json</code>。1.10/1.11 内核不支持 AnyTLS 节点；仅启用 AnyTLS 时不提供这两个旧版订阅。</li>\n\
<li><strong>V2rayN</strong>：粘贴 <code>uri.txt</code>（Base64 URI）或复制 <code>uri</code> 明文分享链接；6.6+ 也可导入 <code>sing-box-full.json</code>。</li>\n\
<li><strong>Shadowrocket（iOS）</strong>：扫码或粘贴 <code>shadowrocket.txt</code> 链接 → 首页添加配置 → 自动更新。五协议均受支持，最低版本：VLESS Reality 2.2.16、TUIC 2.2.12、Hysteria2 2.2.35、AnyTLS 2.2.64；低于对应版本时该协议节点不会被识别（其余节点仍可正常导入）。</li>\n\
</ul>\n\
<script>\n\
function copyUrl(btn, text) {{\n\
  function ok() {{\n\
    var orig = btn.innerText;\n\
    btn.innerText = '已复制 ✓';\n\
    btn.classList.add('btn-success');\n\
    setTimeout(function() {{\n\
      btn.innerText = orig;\n\
      btn.classList.remove('btn-success');\n\
    }}, 2000);\n\
  }}\n\
  if (navigator.clipboard && navigator.clipboard.writeText) {{\n\
    navigator.clipboard.writeText(text).then(ok, function() {{ fallback(); }});\n\
  }} else {{\n\
    fallback();\n\
  }}\n\
  function fallback() {{\n\
    var ta = document.createElement('textarea');\n\
    ta.value = text;\n\
    ta.style.position = 'fixed';\n\
    ta.style.left = '-9999px';\n\
    ta.style.top = '-9999px';\n\
    document.body.appendChild(ta);\n\
    ta.focus();\n\
    ta.select();\n\
    try {{\n\
      if (document.execCommand('copy')) {{ ok(); }}\n\
    }} catch (e) {{}}\n\
    document.body.removeChild(ta);\n\
  }}\n\
}}\n\
</script>\n\
</body>\n</html>\n",
        rotate_command = esc(&rotate_command),
        host = esc(&config.subscription_host),
        mode = esc(&config.subscription_mode.to_string()),
        traffic_badges = traffic_badges(config, traffic.as_ref())?,
        client_rows = client_rows,
        node_rows = node_rows,
        rows = rows,
    ))
}

/// The traffic badges: a short summary when accounting works, an explicit
/// degraded badge when it does not, so the links always stay available.
fn traffic_badges(
    config: &DeploymentConfig,
    traffic: Option<&crate::traffic::TrafficReport>,
) -> Result<String, String> {
    let Some(traffic) = traffic else {
        return Ok(
            "<span class=\"badge\">流量：暂不可用（账期状态恢复后自动显示）</span>".to_owned(),
        );
    };
    let tz: Tz = config
        .client_display_timezone
        .parse()
        .map_err(|_| format!("unknown IANA timezone {}", config.client_display_timezone))?;
    let reset = traffic
        .next_reset
        .with_timezone(&tz)
        .format("%Y-%m-%d %H:%M")
        .to_string();
    Ok(format!(
        "<span class=\"badge\">已用流量：{used}</span>\
<span class=\"badge\">总量：{quota}</span>\
<span class=\"badge\">下次重置：{reset}（{tz}）</span>",
        used = format_gib(traffic.total()),
        quota = if traffic.monthly_traffic_limit > 0 {
            format_gib(traffic.monthly_traffic_limit)
        } else {
            "不限".to_owned()
        },
        reset = esc(&reset),
        tz = esc(&config.client_display_timezone),
    ))
}

fn format_gib(bytes: u64) -> String {
    format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn esc_attr(text: &str) -> String {
    esc(text)
}

/// Re-exported for tests and callers that want to label version rows.
pub fn version_label(version: ClientVersion) -> String {
    format!("sing-box {version}")
}

/// Quotes a configuration name for a shell command shown on the page. Single
/// quotes are the only quoting operator that suppresses every expansion.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn badges_follow_total_adjustments_without_changing_directions() {
        let config = DeploymentConfig::new(
            crate::config::SubscriptionMode::IpFallback,
            "127.0.0.1".into(),
            None,
            Some(2080),
            "ens3".into(),
            vec![crate::config::ManagedProtocol::VlessReality],
            Some("www.cloudflare.com".into()),
        )
        .unwrap();
        let gib = 1024 * 1024 * 1024;
        let mut traffic = crate::traffic::TrafficReport {
            interface: "ens3".into(),
            received: 2 * gib,
            transmitted: 3 * gib,
            total_adjustment: 0,
            monthly_traffic_limit: 10 * gib,
            accounting_period: "test".into(),
            next_reset: chrono::Utc::now(),
        };
        for (adjustment, expected) in [
            (0, "5.00"),
            (i128::from(gib), "6.00"),
            (-i128::from(gib), "4.00"),
            (-5 * i128::from(gib), "0.00"),
        ] {
            traffic.total_adjustment = adjustment;
            assert!(
                traffic_badges(&config, Some(&traffic))
                    .unwrap()
                    .contains(&format!("已用流量：{expected} GiB"))
            );
        }
        traffic.received += gib;
        assert!(
            traffic_badges(&config, Some(&traffic))
                .unwrap()
                .contains("已用流量：1.00 GiB")
        );
    }
}
