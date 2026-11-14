//! Menu and command handlers share typed drafts and one deployment transaction.
use crate::cli::args::{CoreCommand, GroupCommand, NodeCommand};
use sbctl::{
    config::DeploymentStore,
    nodes::{self, Error, Registry, Result},
};
use serde_json::json;
use std::{path::Path, process::ExitCode};
fn finish(result: Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("操作失败：{e}");
            ExitCode::from(2)
        }
    }
}
pub(crate) fn draft(root: &Path) -> Result<(sbctl::config::DeploymentConfig, Registry)> {
    let store = DeploymentStore::new(root);
    let config = store.load()?;
    let registry = nodes::load_or_import(root, &config)?;
    Ok((config, registry))
}
pub(crate) fn publish(
    root: &Path,
    config: &sbctl::config::DeploymentConfig,
    before: &Registry,
    after: Registry,
    apply: bool,
) -> Result<()> {
    let plan = sbctl::deployment::plan(root, config, before, &after)?;
    println!("{}", serde_json::to_string_pretty(&plan).unwrap());
    if apply {
        if !plan.requirements.is_empty() {
            return Err(Error(plan.requirements.join("；")));
        }
        sbctl::deployment::apply(&DeploymentStore::new(root), config, before, after)?;
        println!("已应用并发布完整订阅批次");
    } else {
        println!("仅预览，未应用；添加 --apply 提交。内核迁移可能中断现有连接。");
    }
    Ok(())
}
pub(crate) fn node(root: &Path, command: NodeCommand) -> ExitCode {
    finish((|| {
        let (config, before) = draft(root)?;
        let mut after = before.clone();
        let apply = match command {
            NodeCommand::Probe {
                id,
                url,
                client_bin,
            } => {
                let node = after.node_mut(&id)?;
                let report = sbctl::probe::node(
                    root,
                    &config,
                    node,
                    url.as_deref().unwrap_or(&config.client_latency_probe_url),
                    client_bin.as_deref(),
                )?;
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
                return Ok(());
            }
            NodeCommand::List {
                core,
                preset,
                group,
                enabled,
                json: as_json,
                page,
                per_page,
            } => {
                if page == 0 || per_page == 0 || per_page > 100 {
                    return Err(Error("页码从 1 开始，每页 1–100 项".into()));
                }
                let selected_group = group
                    .as_ref()
                    .map(|id| {
                        before
                            .groups
                            .iter()
                            .find(|g| &g.id == id)
                            .ok_or_else(|| Error("找不到节点组".into()))
                    })
                    .transpose()?;
                let rows: Vec<_> = before
                    .nodes
                    .iter()
                    .filter(|n| {
                        core.is_none_or(|c| c == n.core)
                            && preset.as_ref().is_none_or(|p| p == &n.preset())
                            && enabled.is_none_or(|e| e == n.enabled)
                            && selected_group.is_none_or(|g| g.members.contains(&n.id))
                    })
                    .collect();
                let start = page.saturating_sub(1).saturating_mul(per_page);
                let result:Vec<_>=rows.iter().skip(start).take(per_page).map(|n|json!({"id":n.id,"name":n.name,"tag":n.tag,"core":n.core,"preset":n.preset(),"enabled":n.enabled,"listen":n.listen,"port":n.port(),"credentials":"[隐藏]"})).collect();
                if as_json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &json!({"total":rows.len(),"page":page,"nodes":result})
                        )
                        .unwrap()
                    );
                } else {
                    println!("节点：{} 项，第 {} 页", rows.len(), page);
                    for n in result {
                        println!(
                            "{}  {}  {}  {}  {}:{}  {}",
                            n["id"].as_str().unwrap(),
                            n["name"].as_str().unwrap(),
                            n["preset"].as_str().unwrap(),
                            n["core"].as_str().unwrap(),
                            n["listen"].as_str().unwrap(),
                            n["port"],
                            if n["enabled"] == true {
                                "启用"
                            } else {
                                "停用"
                            }
                        );
                    }
                }
                return Ok(());
            }
            NodeCommand::Migrate { apply } => apply,
            NodeCommand::Add {
                name,
                preset,
                port,
                core,
                server,
                server_name,
                path,
                apply,
            } => {
                let mut node = nodes::create(&after, &config, &preset, &name, port, core)?;
                for (key, value) in [
                    ("server", server),
                    ("server_name", server_name),
                    ("path", path),
                ] {
                    if let Some(v) = value {
                        node.patch(key, json!(v))?;
                    }
                }
                println!("新增节点 ID：{}", node.id);
                after.nodes.push(node);
                apply
            }
            NodeCommand::Edit {
                id,
                name,
                port,
                server,
                server_name,
                path,
                listen,
                input,
                apply,
            } => {
                let node = after.node_mut(&id)?;
                if let Some(path) = input {
                    let bytes = sbctl::config::read_managed_file_no_follow(&path)
                        .map_err(|_| Error("无法读取私有节点文件".into()))?;
                    if bytes.len() > 1024 * 1024 {
                        return Err(Error("节点文件超过 1 MiB".into()));
                    }
                    node.data = serde_json::from_slice(&bytes)
                        .map_err(|_| Error("节点文件不是合法的类型化 Data 文档".into()))?;
                }
                if let Some(name) = name {
                    node.name = name;
                }
                if let Some(listen) = listen {
                    node.listen = listen;
                }
                if let Some(port) = port {
                    node.patch("port", json!(port))?;
                }
                for (key, value) in [
                    ("server", server),
                    ("server_name", server_name),
                    ("path", path),
                ] {
                    if let Some(v) = value {
                        node.patch(key, json!(v))?;
                    }
                }
                apply
            }
            NodeCommand::Copy {
                id,
                name,
                port,
                apply,
            } => {
                let mut node = after.node_mut(&id)?.clone();
                let id = nodes::new_id();
                node.id = id.clone();
                node.tag = format!("node-{id}");
                node.name = name;
                node.renew_secrets()?;
                let port = port
                    .map(Ok)
                    .unwrap_or_else(|| nodes::free_port(&after, &config))?;
                node.patch("port", json!(port))?;
                after.nodes.push(node);
                apply
            }
            NodeCommand::Enable { id, apply } => {
                after.node_mut(&id)?.enabled = true;
                apply
            }
            NodeCommand::Disable { id, apply } => {
                after.node_mut(&id)?.enabled = false;
                apply
            }
            NodeCommand::Move { id, core, apply } => {
                after.node_mut(&id)?.core = core;
                apply
            }
            NodeCommand::Remove { id, apply } => {
                after.node_mut(&id)?;
                if after.groups.iter().any(|g| g.members.contains(&id)) {
                    return Err(Error("节点仍被分组引用，请先修改组成员".into()));
                }
                after.nodes.retain(|n| n.id != id);
                apply
            }
            NodeCommand::Check { id } => {
                if let Some(id) = id {
                    after.node_mut(&id)?.validate_data()?;
                }
                let docs = sbctl::subscription::managed::servers(&config, root, &after)
                    .map_err(|e| Error(e.to_string()))?;
                let mut checked = 0;
                for (core, doc) in docs {
                    let path = root.join(sbctl::deployment::binary(core));
                    if path.is_file() {
                        core.check_config(&path, &doc.to_string())
                            .map_err(|_| Error(format!("{core} 配置检查失败")))?;
                        checked += 1;
                    }
                }
                println!(
                    "节点结构检查通过；执行了 {checked} 项真实内核配置检查。连接状态请结合 core status 和客户端真实转发验证。"
                );
                return Ok(());
            }
            NodeCommand::Export { id, qr } => {
                let node = after.node_mut(&id)?;
                let link = sbctl::subscription::managed::share_link(&config, node, false)
                    .map_err(|e| Error(e.to_string()))?;
                print!("{link}");
                if qr {
                    print!("{}", sbctl::qr::render_ansi(link.trim()).map_err(Error)?);
                }
                return Ok(());
            }
        };
        publish(root, &config, &before, after, apply)
    })())
}
pub(crate) fn group(root: &Path, command: GroupCommand) -> ExitCode {
    finish((|| {
        let (config, before) = draft(root)?;
        let mut after = before.clone();
        let apply = match command {
            GroupCommand::List => {
                for g in before.groups {
                    println!(
                        "{}  {}  {:?}  {} 节点",
                        g.id,
                        g.name,
                        g.strategy,
                        g.members.len()
                    );
                }
                return Ok(());
            }
            GroupCommand::Create {
                name,
                members,
                strategy,
                apply,
            } => {
                let id = nodes::new_id();
                println!("新分组 ID：{id}");
                after.groups.push(nodes::Group {
                    id,
                    name,
                    members,
                    strategy,
                });
                apply
            }
            GroupCommand::Edit {
                id,
                name,
                members,
                strategy,
                apply,
            } => {
                let g = after
                    .groups
                    .iter_mut()
                    .find(|g| g.id == id)
                    .ok_or_else(|| Error("找不到分组 ID".into()))?;
                if let Some(name) = name {
                    g.name = name;
                }
                if let Some(members) = members {
                    g.members = members;
                }
                if let Some(strategy) = strategy {
                    g.strategy = strategy;
                }
                apply
            }
            GroupCommand::Remove { id, apply } => {
                if !after.groups.iter().any(|g| g.id == id) {
                    return Err(Error("找不到分组 ID".into()));
                }
                after.groups.retain(|g| g.id != id);
                apply
            }
            GroupCommand::Links {
                id,
                format,
                credential_name,
                qr,
            } => {
                links(
                    root,
                    &config,
                    id.as_deref(),
                    format,
                    credential_name.as_deref(),
                    qr,
                )?;
                return Ok(());
            }
        };
        publish(root, &config, &before, after, apply)
    })())
}
pub(crate) fn links(
    root: &Path,
    config: &sbctl::config::DeploymentConfig,
    id: Option<&str>,
    format: Option<sbctl::subscription::SubscriptionFormat>,
    credential_name: Option<&str>,
    qr: bool,
) -> Result<()> {
    let generation = sbctl::deployment::current(root)?
        .ok_or_else(|| Error("请先执行 node migrate --apply".into()))?;
    let catalog = sbctl::subscription::managed::catalog(root, &generation)
        .map_err(|e| Error(e.to_string()))?;
    if let Some(id) = id
        && !catalog.groups.iter().any(|g| g.id == id)
    {
        return Err(Error("找不到已发布节点组".into()));
    }
    let credential = if let Some(name) = credential_name {
        config
            .subscription_credentials
            .iter()
            .find(|c| c.name == name && c.is_active(chrono::Utc::now().timestamp()))
            .map(|c| c.credential.as_str())
            .ok_or_else(|| Error("设备凭据不存在或已过期".into()))?
    } else {
        &config.subscription_credential
    };
    let mut count = 0;
    for row in sbctl::subscription::subscription_matrix() {
        if format.is_some_and(|f| f != row.format) {
            continue;
        }
        let report = catalog
            .reports
            .iter()
            .find(|r| r.group.as_deref() == id && r.format == row.format.path_name())
            .ok_or_else(|| Error("订阅目录缺少格式报告".into()))?;
        if report.included == 0 {
            if format.is_some() {
                return Err(Error("目标格式没有兼容节点".into()));
            }
            continue;
        }
        let url = if let Some(id) = id {
            sbctl::subscription::managed::group_url(config, credential, id, row.format)
        } else {
            sbctl::subscription::managed::format_url(config, credential, row.format)
        }
        .map_err(|e| Error(e.to_string()))?;
        println!(
            "{}：包含 {}，排除 {}\n{url}",
            row.label,
            report.included,
            report.excluded.len()
        );
        for (node, reason) in &report.excluded {
            println!("  {node}：{reason}");
        }
        if qr {
            print!(
                "{}",
                sbctl::qr::render_ansi(&row.format.qr_payload(&url)).map_err(Error)?
            );
        }
        count += 1;
    }
    if count == 0 {
        return Err(Error("没有可用订阅".into()));
    }
    Ok(())
}
pub(crate) fn core(root: &Path, command: CoreCommand) -> ExitCode {
    finish((|| {
        let store = DeploymentStore::new(root);
        match command {
            CoreCommand::Status => println!(
                "{}",
                serde_json::to_string_pretty(&sbctl::deployment::status(root)?).unwrap()
            ),
            CoreCommand::Capabilities => println!(
                "sing-box：VLESS Reality / VMess WS / Hy2 / TUIC / AnyTLS / VLESS WS TLS / Trojan TCP TLS\nXray 26.10.31：VLESS Reality / VMess WS / VLESS XHTTP Reality / VLESS XHTTP TLS / VLESS WS TLS / Trojan TCP TLS\n客户端兼容性独立计算；XHTTP 与 Reality 受目标版本限制。"
            ),
            CoreCommand::Mode {
                mode,
                disable,
                apply,
            } => {
                let (config, before) = draft(root)?;
                let mut after = before.clone();
                after.switch(mode, &disable)?;
                publish(root, &config, &before, after, apply)?;
            }
            CoreCommand::Apply {
                client_version,
                apply,
            } => {
                let (config, before) = draft(root)?;
                let mut after = before.clone();
                if let Some(v) = client_version {
                    after.client_version = v;
                }
                publish(root, &config, &before, after, apply)?;
            }
            CoreCommand::Recover { boot } => println!(
                "恢复检查完成：{}",
                store.root().display().to_string()
                    + if sbctl::deployment::recover(&store, boot)? {
                        "（已处理未完成事务）"
                    } else {
                        "（无未完成事务）"
                    }
            ),
            CoreCommand::Rollback => {
                sbctl::deployment::rollback(&store)?;
                println!("已恢复最近部署；订阅凭据和流量账期未回退");
            }
            CoreCommand::Install { core, version } | CoreCommand::Update { core, version } => {
                sbctl::deployment::install_core(&store, core, version.as_deref())?;
                println!("{core} 已校验并安装");
            }
            CoreCommand::Restart { core } => {
                sbctl::deployment::restart(&store, core)?;
                println!("已校验并重启所选内核");
            }
        }
        Ok(())
    })())
}
