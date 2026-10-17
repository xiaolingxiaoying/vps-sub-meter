//! One catalog drives both displayed numbering and dispatch. It is shared with
//! the menu coverage tests, so adding a label cannot leave an unmapped option.
use sbctl::subscription::SubscriptionFormat;
use sbctl::wizard::ConfigurationTopic;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Page {
    Home,
    Overview,
    Deployment,
    Subscriptions,
    Formats,
    Credentials,
    Traffic,
    Services,
    Advanced,
    Overrides,
    Rules,
    Email,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Open(Page),
    Status,
    Logs,
    KernelStatus,
    Connections,
    CertificateStatus,
    QuickInstall,
    GuidedInstall,
    Nodes,
    NodeLinks,
    Topic(ConfigurationTopic),
    Template,
    SubscriptionUrls,
    Qr(SubscriptionFormat),
    CredentialList,
    CredentialAdd,
    CredentialRevoke,
    Rotate,
    Traffic,
    TotalCorrection,
    DirectionCorrection,
    Reset,
    Restart,
    ApiEnable,
    ApiDisable,
    CertificateObtain,
    CertificateRenew,
    Update,
    KernelUpdate,
    EditServer,
    EditSingBox,
    EditClash,
    OverrideShow,
    OverrideValidate,
    Preview,
    Export,
    FullWizard,
    Regenerate,
    Reinstall,
    Uninstall,
    Purge,
    RuleList,
    RuleAdd,
    RuleRemove,
    RuleSetList,
    RuleSetAdd,
    RuleSetRemove,
    RuleImport,
    RuleInterval,
    EmailConfigure,
    EmailStatus,
    EmailSend,
    EmailEnable,
    EmailDisable,
}
pub(super) struct Entry {
    pub label: String,
    pub action: Action,
}
fn entry(label: &str, action: Action) -> Entry {
    Entry {
        label: label.into(),
        action,
    }
}
pub(super) fn title(page: Page) -> &'static str {
    match page {
        Page::Home => "主菜单",
        Page::Overview => "概览与诊断",
        Page::Deployment => "部署与协议",
        Page::Subscriptions => "订阅与客户端",
        Page::Formats => "订阅与客户端 / 链接与二维码",
        Page::Credentials => "订阅与客户端 / 命名订阅凭据",
        Page::Traffic => "流量与通知",
        Page::Services => "服务与更新",
        Page::Advanced => "高级与维护",
        Page::Overrides => "高级与维护 / 配置与覆写",
        Page::Rules => "高级与维护 / 路由规则",
        Page::Email => "流量与通知 / 邮件通知",
    }
}
pub(super) fn entries(page: Page, installed: bool) -> Vec<Entry> {
    use Action::*;
    use Page::{
        Advanced, Credentials, Deployment, Email, Formats, Home, Overrides, Overview, Rules,
        Services, Subscriptions,
    };
    match page {
        Home => vec![
            entry("概览与诊断", Open(Overview)),
            entry("部署与协议", Open(Deployment)),
            entry("订阅与客户端", Open(Subscriptions)),
            entry("流量与通知", Open(Page::Traffic)),
            entry("服务与更新", Open(Services)),
            entry("高级与维护", Open(Advanced)),
        ],
        Overview => vec![
            entry("完整部署状态", Status),
            entry("运行日志", Logs),
            entry("内核版本、进程和资源状态", KernelStatus),
            entry("当前连接", Connections),
            entry("证书状态", CertificateStatus),
        ],
        Deployment => {
            let mut result = Vec::new();
            if !installed {
                result.extend([
                    entry("快速安装（默认五协议，可跳过定制）", QuickInstall),
                    entry("引导安装（逐项配置）", GuidedInstall),
                ]);
            }
            result.extend([
                entry("查看节点与监听端口", Nodes),
                entry(
                    "协议启停、端口、SNI 和证书",
                    Topic(ConfigurationTopic::Protocols),
                ),
                entry("节点分享链接（含凭据，需确认）", NodeLinks),
            ]);
            result
        }
        Subscriptions => vec![
            entry("切换客户端模板（精简 / 标准）", Template),
            entry("查看订阅链接 / 按格式显示二维码", Open(Formats)),
            entry("配置订阅入口", Topic(ConfigurationTopic::Subscription)),
            entry("命名订阅凭据（按设备管理）", Open(Credentials)),
            entry("轮换默认订阅凭据（旧默认链接立即失效）", Rotate),
            entry(
                "高级客户端设置（DNS / 规则来源 / 测速）",
                Topic(ConfigurationTopic::ClientTemplate),
            ),
        ],
        Formats => {
            let mut result = vec![entry("查看全部订阅链接", SubscriptionUrls)];
            for row in sbctl::subscription::subscription_matrix() {
                result.push(entry(
                    &format!("{} 二维码", row.format.path_name()),
                    Qr(row.format),
                ));
            }
            result
        }
        Credentials => vec![
            entry("查看凭据（密钥脱敏）", CredentialList),
            entry("新增设备凭据", CredentialAdd),
            entry("吊销指定凭据", CredentialRevoke),
        ],
        Page::Traffic => vec![
            entry("查看本周期流量", Action::Traffic),
            entry(
                "配置额度、账期、时区与网卡",
                Topic(ConfigurationTopic::Traffic),
            ),
            entry("修正本周期总流量（GiB）", TotalCorrection),
            entry("修正本周期 RX/TX（GiB）", DirectionCorrection),
            entry("立即建立 / 刷新账期（需确认）", Reset),
            entry("邮件通知设置", Open(Email)),
        ],
        Services => vec![
            entry("校验配置并重启服务", Restart),
            entry("启用本机连接观察 API", ApiEnable),
            entry("关闭连接观察 API", ApiDisable),
            entry("签发 / 修复 HTTPS 证书", CertificateObtain),
            entry("续期 HTTPS 证书", CertificateRenew),
            entry("更新 sbctl（签名 Release）", Update),
            entry("更新 / 切换 sing-box 内核", KernelUpdate),
        ],
        Advanced => vec![
            entry("客户端与服务端文件覆写 / 预览 / 导出", Open(Overrides)),
            entry("本地规则与远程规则集", Open(Rules)),
            entry("完整配置向导", FullWizard),
            entry("重新生成并校验配置工件", Regenerate),
            entry("备份旧部署并重新安装", Reinstall),
            entry("卸载服务和二进制（保留备份）", Uninstall),
            entry("彻底清理 sbctl 数据（双重确认）", Purge),
        ],
        Overrides => vec![
            entry("编辑服务端 JSON 覆写", EditServer),
            entry("编辑客户端 sing-box 覆写", EditSingBox),
            entry("编辑客户端 Mihomo 覆写", EditClash),
            entry("查看覆写文件与合并规则", OverrideShow),
            entry("校验全部覆写", OverrideValidate),
            entry("预览配置（含凭据，需确认）", Preview),
            entry("导出配置到私有新文件", Export),
        ],
        Rules => vec![
            entry("查看本地规则", RuleList),
            entry("添加本地规则", RuleAdd),
            entry("移除本地规则", RuleRemove),
            entry("查看远程规则集", RuleSetList),
            entry("注册远程规则集", RuleSetAdd),
            entry("移除远程规则集", RuleSetRemove),
            entry("导入文本规则列表", RuleImport),
            entry("设置规则集刷新间隔", RuleInterval),
        ],
        Email => vec![
            entry("配置 SMTP、收件人和定时通知", EmailConfigure),
            entry("查看邮件配置（密码隐藏）", EmailStatus),
            entry("立即发送报告（需确认）", EmailSend),
            entry("启用定时邮件", EmailEnable),
            entry("关闭定时邮件", EmailDisable),
        ],
    }
}
pub(super) fn select(entries: &[Entry], input: &str) -> Option<Action> {
    let index = input.parse::<usize>().ok()?.checked_sub(1)?;
    entries.get(index).map(|entry| entry.action)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn six_sections_and_every_nested_choice_share_the_numbering_catalog() {
        let home = entries(Page::Home, false);
        assert_eq!(home.len(), 6);
        let mut pending = vec![Page::Home];
        let mut seen = Vec::new();
        let mut leaves = Vec::new();
        while let Some(page) = pending.pop() {
            assert!(
                !seen.contains(&page),
                "navigation cycle or duplicate page: {page:?}"
            );
            seen.push(page);
            let rows = entries(page, false);
            assert!(select(&rows, "0").is_none());
            for invalid in ["-1", "abc", "99999999999999999999999999999"] {
                assert!(select(&rows, invalid).is_none());
            }
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(select(&rows, &(i + 1).to_string()), Some(row.action));
                assert!(!row.label.is_empty());
                if let Action::Open(next) = row.action {
                    pending.push(next);
                } else {
                    assert!(
                        !leaves.contains(&row.action),
                        "duplicate action {row:?}",
                        row = row.label
                    );
                    leaves.push(row.action);
                }
            }
        }
        assert_eq!(seen.len(), 12);
        assert_eq!(
            leaves
                .iter()
                .filter(|a| matches!(a, Action::Regenerate))
                .count(),
            1
        );
        assert!(leaves.contains(&Action::Template));
        assert!(leaves.contains(&Action::Purge));
    }
    #[test]
    fn deployed_menu_never_offers_an_accidental_fresh_install() {
        let rows = entries(Page::Deployment, true);
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.action, Action::QuickInstall | Action::GuidedInstall))
        );
        assert!(
            rows.iter()
                .any(|r| r.action == Action::Topic(ConfigurationTopic::Protocols))
        );
    }
}
