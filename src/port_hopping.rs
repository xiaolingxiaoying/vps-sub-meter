//! Opt-in Hy2 client hopping and an isolated, atomic nftables redirect table.
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::config::{ConfigError, DeploymentConfig, DeploymentStore, ManagedProtocol};

const TABLE: &str = "sbctl_hy2";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PortHopping {
    pub start: u16,
    pub end: u16,
    pub interval_seconds: u16,
}

impl PortHopping {
    pub fn range(&self) -> String {
        format!("{}-{}", self.start, self.end)
    }
}

pub fn validate(config: &DeploymentConfig) -> Result<(), ConfigError> {
    let Some(hop) = &config.hysteria2_port_hopping else {
        return Ok(());
    };
    if !config
        .enabled_protocols
        .contains(&ManagedProtocol::Hysteria2)
        || config.hysteria2.is_none()
    {
        return Err(ConfigError::InvalidValue("端口跳跃需要先启用 Hysteria2"));
    }
    if hop.start < 10000 || hop.start >= hop.end || !(5..=3600).contains(&hop.interval_seconds) {
        return Err(ConfigError::InvalidValue(
            "跳跃范围必须为 10000–65535 内至少两个端口，间隔为 5–3600 秒",
        ));
    }
    // Reserve the range across both transports, just like managed listener ports.
    let other_ports = [
        config.vless_reality.as_ref().map(|n| n.listen_port),
        config.vmess_websocket.as_ref().map(|n| n.listen_port),
        config.tuic.as_ref().map(|n| n.listen_port),
        config.anytls.as_ref().map(|n| n.listen_port),
        config.http_port,
        config.subscription_listen_port,
        config.server_clash_api.as_ref().map(|a| a.port),
    ];
    if other_ports
        .into_iter()
        .flatten()
        .any(|p| (hop.start..=hop.end).contains(&p))
    {
        return Err(ConfigError::InvalidValue(
            "跳跃范围不能覆盖其他协议、订阅或 API 监听端口",
        ));
    }
    Ok(())
}

fn rules(config: &DeploymentConfig, replace: bool) -> String {
    let hop = config
        .hysteria2_port_hopping
        .as_ref()
        .expect("validated hopping");
    let port = config
        .hysteria2
        .as_ref()
        .expect("validated Hy2")
        .listen_port;
    let delete = if replace {
        format!("delete table inet {TABLE}\n")
    } else {
        String::new()
    };
    format!(
        "{delete}table inet {TABLE} {{\n chain prerouting {{\n type nat hook prerouting priority dstnat; policy accept;\n iifname \"{}\" fib daddr type local udp dport {}-{} counter redirect to :{port}\n }}\n}}\n",
        config.interface, hop.start, hop.end
    )
}

/// Called only by the root systemd pre-start/post-stop hooks. Never flushes the
/// host ruleset. nft applies deletion and replacement in one transaction.
pub fn apply_firewall(root: &Path, cleanup: bool) -> Result<(), String> {
    #[cfg(unix)]
    if unsafe { libc::geteuid() } != 0 {
        return Err("Hy2 firewall hook requires root".into());
    }
    let config = if cleanup {
        None
    } else {
        let config = DeploymentStore::new(root)
            .load()
            .map_err(|e| e.to_string())?;
        config.validate().map_err(|e| e.to_string())?;
        Some(config)
    };
    let enabled = config
        .as_ref()
        .is_some_and(|c| c.hysteria2_port_hopping.is_some());
    let runtime = crate::runtime::Runtime::live(root);
    let listed = runtime.run_command_output("nft", &["list", "tables"]);
    let (status, tables) = match listed {
        Ok(result) => result,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !enabled => return Ok(()),
        Err(e) => return Err(format!("端口跳跃需要 nftables，请先安装 nftables：{e}")),
    };
    if !status.success() {
        return Err(format!("无法读取 nftables：{tables}"));
    }
    let exists = tables
        .lines()
        .any(|line| line.trim() == format!("table inet {TABLE}"));
    let script = if enabled {
        rules(config.as_ref().unwrap(), exists)
    } else if exists {
        format!("delete table inet {TABLE}\n")
    } else {
        return Ok(());
    };
    let mut child = runtime
        .command("nft", &["-f", "-"])
        .map_err(|e| e.to_string())?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let write = child.stdin.take().unwrap().write_all(script.as_bytes());
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    write.map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Hy2 nftables 事务失败：{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub fn configure<C: crate::wizard::Prompts>(
    config: &mut DeploymentConfig,
    prompts: &mut C,
) -> Result<(), crate::wizard::WizardError> {
    use crate::wizard::WizardError;
    if !config
        .enabled_protocols
        .contains(&ManagedProtocol::Hysteria2)
    {
        return Err(WizardError::Invalid(
            "请先在协议设置中启用 Hysteria2".into(),
        ));
    }
    prompts.report("Hy2 端口跳跃：需安装 nftables 并在云安全组开放所选 UDP 范围。服务端自动转发，停止服务自动清理；sing-box 1.10 订阅保留固定端口。");
    loop {
        let default = if config.hysteria2_port_hopping.is_some() {
            "1"
        } else {
            "2"
        };
        match prompts
            .ask("端口跳跃（1 启用 / 2 关闭 / 0 取消）", Some(default))?
            .as_str()
        {
            "0" => return Ok(()),
            "2" => {
                config.hysteria2_port_hopping = None;
                return Ok(());
            }
            "1" => break,
            "" if default == "2" => {
                config.hysteria2_port_hopping = None;
                return Ok(());
            }
            "" => break,
            _ => prompts.report("请输入 0、1 或 2"),
        }
    }
    loop {
        let current = config
            .hysteria2_port_hopping
            .clone()
            .unwrap_or(PortHopping {
                start: 20000,
                end: 30000,
                interval_seconds: 30,
            });
        let range = prompts.ask(
            "跳跃端口（1 20000-30000 / 2 40000-50000 / 或输入 起始-结束）",
            Some(&current.range()),
        )?;
        let range = match range.as_str() {
            "" => current.range(),
            "1" => "20000-30000".into(),
            "2" => "40000-50000".into(),
            _ => range,
        };
        let parsed = range
            .split_once('-')
            .and_then(|(a, b)| Some((a.parse::<u16>().ok()?, b.parse::<u16>().ok()?)));
        let Some((start, end)) = parsed else {
            prompts.report("请输入有效范围，例如 20000-30000");
            continue;
        };
        let interval = prompts.ask(
            "跳跃间隔（1 30秒 / 2 60秒 / 或输入 5-3600 秒）",
            Some(&current.interval_seconds.to_string()),
        )?;
        let interval = match interval.as_str() {
            "" => Some(current.interval_seconds),
            "1" => Some(30),
            "2" => Some(60),
            _ => interval.parse().ok(),
        };
        let Some(interval_seconds) = interval else {
            prompts.report("请输入有效秒数");
            continue;
        };
        let previous = config.hysteria2_port_hopping.replace(PortHopping {
            start,
            end,
            interval_seconds,
        });
        match validate(config) {
            Ok(()) => return Ok(()),
            Err(e) => {
                config.hysteria2_port_hopping = previous;
                prompts.report(&e.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> DeploymentConfig {
        let mut c = DeploymentConfig::new(
            crate::config::SubscriptionMode::IpFallback,
            "127.0.0.1".into(),
            None,
            Some(2080),
            "lo".into(),
            vec![ManagedProtocol::Hysteria2],
            None,
        )
        .unwrap();
        c.hysteria2_port_hopping = Some(PortHopping {
            start: 20000,
            end: 20100,
            interval_seconds: 30,
        });
        c
    }

    #[test]
    fn rejects_bad_ranges_intervals_and_overlapping_listeners() {
        let mut c = config();
        c.validate().unwrap();
        for hop in [
            PortHopping {
                start: 9999,
                end: 20000,
                interval_seconds: 30,
            },
            PortHopping {
                start: 20000,
                end: 20000,
                interval_seconds: 30,
            },
            PortHopping {
                start: 20000,
                end: 20100,
                interval_seconds: 4,
            },
            PortHopping {
                start: 20000,
                end: 20100,
                interval_seconds: 3601,
            },
        ] {
            c.hysteria2_port_hopping = Some(hop);
            assert!(c.validate().is_err());
        }
        c = config();
        c.http_port = Some(20050);
        assert!(c.validate().is_err());
        c = config();
        c.enabled_protocols.clear();
        assert!(c.validate().is_err());
    }

    #[test]
    fn firewall_owns_only_local_udp_on_the_configured_ingress() {
        let c = config();
        let script = rules(&c, true);
        assert!(script.starts_with("delete table inet sbctl_hy2\n"));
        assert!(script.contains("iifname \"lo\" fib daddr type local udp dport 20000-20100"));
        assert!(script.contains(&format!(
            "redirect to :{}",
            c.hysteria2.as_ref().unwrap().listen_port
        )));
        assert!(!script.contains("flush ruleset"));
    }

    #[test]
    fn subscriptions_share_ranges_and_legacy_profiles_keep_a_fixed_port() {
        use crate::subscription::{
            SING_BOX_VERSION_PROFILES, SubscriptionFormat, generated_artifacts,
        };
        let c = config();
        let root = tempfile::tempdir().unwrap();
        let artifacts = generated_artifacts(&c, root.path()).unwrap();
        let contents = |name: &str| {
            artifacts
                .iter()
                .find(|(n, _)| n == name)
                .unwrap()
                .1
                .as_str()
        };
        for profile in SING_BOX_VERSION_PROFILES {
            let name = SubscriptionFormat::SingBoxVersion(profile.version).artifact_name();
            let value: serde_json::Value = serde_json::from_str(contents(&name)).unwrap();
            let hy2 = value["outbounds"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["type"] == "hysteria2")
                .unwrap();
            if profile.version.minor < 11 {
                assert!(hy2.get("server_ports").is_none());
                assert_eq!(
                    hy2["server_port"],
                    c.hysteria2.as_ref().unwrap().listen_port
                );
            } else {
                assert!(hy2.get("server_port").is_none());
                assert_eq!(hy2["server_ports"], serde_json::json!(["20000:20100"]));
                assert_eq!(hy2["hop_interval"], "30s");
            }
        }
        let clash: serde_json::Value =
            serde_yaml::from_str(contents("subscription-clash.yaml")).unwrap();
        assert_eq!(clash["proxies"][0]["ports"], "20000-20100");
        assert_eq!(clash["proxies"][0]["hop-interval"], 30);
        let link = crate::subscription::node_share_link(&c, &crate::canonical::nodes(&c)[0]);
        assert!(link.contains(":20000-20100?"));
        assert_eq!(link, contents("subscription-uri.txt"));
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(contents("subscription-shadowrocket.txt"))
            .unwrap();
        assert!(
            String::from_utf8(decoded)
                .unwrap()
                .contains(":20000-20100?")
        );
    }
}
