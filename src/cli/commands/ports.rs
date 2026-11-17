use crate::cli::args::PortCommand;
use sbctl::{
    config::DeploymentStore,
    nodes::{self, Error},
    shared_tls::Route,
};
use std::{path::Path, process::ExitCode};
pub(crate) fn run(root: &Path, command: PortCommand) -> ExitCode {
    let result = (|| -> nodes::Result<()> {
        let store = DeploymentStore::new(root);
        let old = store.load()?;
        let before = nodes::load_or_import(root, &old)?;
        let mut config = old.clone();
        let mut after = before.clone();
        if matches!(command, PortCommand::Status) {
            println!(
                "订阅：{}；地址 {}\nTCP 和 UDP 分开占用；两个 TCP 服务共用 443 时，必须使用不同 SNI。",
                old.subscription_mode, old.subscription_host
            );
            for n in &before.nodes {
                println!(
                    "{} | {} | 公网 {} | 实际监听 {} | {}",
                    n.name,
                    if sbctl::catalog::udp(&n.preset()) {
                        "UDP"
                    } else {
                        "TCP"
                    },
                    n.port(),
                    sbctl::shared_tls::binding(&old, n),
                    if n.enabled { "启用" } else { "停用" }
                );
            }
            for route in old.shared_tls.iter().flat_map(|s| &s.routes) {
                println!("TCP 443 / {} → {}", route.server_name, route.backend);
            }
            return Ok(());
        }
        sbctl::preflight::require_install_privileges(root).map_err(|e| Error(e.to_string()))?;
        let apply = match command {
            PortCommand::Share {
                node,
                website_sni,
                website_port,
                apply,
            } => {
                if node.is_none() && website_sni.is_none() {
                    return Err(Error(
                        "请指定 --node 或 --website-sni/--website-port".into(),
                    ));
                }
                let mut shared = config.shared_tls.clone().unwrap_or_default();
                if let Some(id) = node {
                    let existing = after
                        .nodes
                        .iter()
                        .find(|n| n.id == id)
                        .ok_or_else(|| Error("节点不存在".into()))?;
                    if sbctl::catalog::udp(&existing.preset()) {
                        return Err(Error("Hysteria2 / TUIC 是 UDP：直接将节点端口设为 443，可与网站 TCP 443 共存，无需 TLS 分流".into()));
                    }
                    let backend_port = shared.node_route(&id).map(|r| r.backend.port()).unwrap_or(
                        if existing.port() >= 10000 {
                            existing.port()
                        } else {
                            nodes::free_port(&after, &config)?
                        },
                    );
                    let server_name = sbctl::shared_tls::sni(existing).to_string();
                    shared.routes.retain(|r| r.node_id.as_deref() != Some(&id));
                    shared.routes.push(Route {
                        server_name,
                        backend: ([127, 0, 0, 1], backend_port).into(),
                        node_id: Some(id.clone()),
                    });
                    after.node_mut(&id)?.patch("port", serde_json::json!(443))?;
                }
                if let (Some(name), Some(port)) = (website_sni, website_port) {
                    // Never silently overwrite a node's route with a website.
                    if shared
                        .routes
                        .iter()
                        .any(|r| r.server_name.eq_ignore_ascii_case(&name) && r.node_id.is_some())
                    {
                        return Err(Error("网站 SNI 与节点冲突；请使用不同名称".into()));
                    }
                    shared
                        .routes
                        .retain(|r| !r.server_name.eq_ignore_ascii_case(&name));
                    shared.routes.push(Route {
                        server_name: name,
                        backend: ([127, 0, 0, 1], port).into(),
                        node_id: None,
                    });
                }
                config.shared_tls = Some(shared);
                apply
            }
            PortCommand::Remove { server_name, apply } => {
                let shared = config
                    .shared_tls
                    .as_mut()
                    .ok_or_else(|| Error("未启用 443 分流".into()))?;
                let route = shared
                    .routes
                    .iter()
                    .find(|r| r.server_name.eq_ignore_ascii_case(&server_name))
                    .cloned()
                    .ok_or_else(|| Error("分流规则不存在".into()))?;
                if let Some(id) = &route.node_id {
                    after
                        .node_mut(id)?
                        .patch("port", serde_json::json!(route.backend.port()))?;
                }
                shared
                    .routes
                    .retain(|r| !r.server_name.eq_ignore_ascii_case(&server_name));
                if shared.routes.is_empty() {
                    config.shared_tls = None;
                }
                apply
            }
            PortCommand::Status => unreachable!(),
        };
        config.validate()?;
        after.validate(&config)?;
        let plan = sbctl::deployment::plan(root, &config, &before, &after)?;
        println!("{}", serde_json::to_string_pretty(&plan).unwrap());
        println!(
            "443 分流：{}",
            serde_json::to_string_pretty(&config.shared_tls).unwrap()
        );
        println!(
            "网站需自行监听本机 HTTPS 后端端口，使用不同域名/SNI；本工具不修改其他应用配置。TLS 透传不支持依靠 HTTP 路径区分相同 SNI。"
        );
        if apply {
            sbctl::deployment::apply_config_and_nodes(&store, &old, &config, &before, after)?;
            println!("端口与订阅已提交；新连接使用当前分流规则。");
        } else {
            println!("仅预览；添加 --apply 提交。");
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("端口配置失败：{error}");
            ExitCode::from(2)
        }
    }
}
