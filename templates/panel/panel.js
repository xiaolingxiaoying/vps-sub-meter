"use strict";

const byId = (id) => document.getElementById(id);
const pages = [
  ["overview", "概览"], ["deployment", "部署配置"], ["nodes", "节点端口"],
  ["traffic", "流量账期"], ["subscriptions", "订阅链接"], ["tokens", "设备凭据"],
  ["domain", "域名与 HTTPS"], ["argo", "Argo 隧道"], ["certificate", "证书管理"],
  ["maintenance", "系统维护"],
];
const nodeTypes = [
  ["vless_reality_pqc", "VLESS REALITY PQC", "TCP", "Xray"],
  ["vless_xhttp_pqc", "VLESS XHTTP PQC", "TCP", "Xray"],
  ["anytls_tcp", "AnyTLS", "TCP", "sing-box"],
  ["hysteria2_hop", "Hysteria2", "UDP", "sing-box"],
  ["tuic_bbr", "TUIC", "UDP", "sing-box"],
];
const subscriptionFormats = [
  ["index", "订阅看板", false], ["clash", "Clash / Mihomo", true],
  ["sing_box", "sing-box", true], ["shadowrocket", "Shadowrocket", true], ["uri", "URI", false],
];
const commonTimeZones = [
  ["Asia/Shanghai", "北京时间 · 上海"],
  ["America/Los_Angeles", "美西 · 洛杉矶"],
  ["America/New_York", "美东 · 纽约"],
  ["America/Chicago", "美中 · 芝加哥"],
  ["America/Denver", "美国山地 · 丹佛"],
  ["Asia/Hong_Kong", "香港"],
  ["Asia/Taipei", "台北"],
  ["Asia/Tokyo", "日本 · 东京"],
  ["Asia/Singapore", "新加坡"],
  ["Europe/London", "英国 · 伦敦"],
  ["Europe/Berlin", "中欧 · 柏林"],
  ["UTC", "协调世界时 · UTC"],
];
const drafts = new Map();
let key = "";
let model;
let trafficUsageDraft;
let selectedSubscriptionDevice = "";
let busy = false;
let panelClosed = false;

function element(tag, attributes = {}, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) {
    if (name === "class") node.className = value;
    else if (name.startsWith("on")) node.addEventListener(name.slice(2), value);
    else if (value === true) node.setAttribute(name, "");
    else if (value !== false && value != null) node.setAttribute(name, value);
  }
  node.append(...children.flat().filter((child) => child != null && child !== false));
  return node;
}

function button(label, fn, { primary = false, danger = false, disabled = false } = {}) {
  return element("button", {
    type: "button", class: primary ? "primary" : danger ? "danger" : "",
    disabled, onclick: () => {
      if (busy || panelClosed) return;
      void Promise.resolve().then(fn).catch(showError);
    },
  }, label);
}

function card(title, ...children) {
  return element("section", { class: "card" }, element("h2", {}, title), ...children);
}

function actions(...children) { return element("div", { class: "actions" }, ...children); }
function grid(...children) { return element("div", { class: "form-grid" }, ...children); }
function stack(...children) { return element("div", { class: "stack" }, ...children); }
function text(value) { return value == null || value === "" ? "—" : String(value); }
function callout(value, error = false) { return element("p", { class: `callout${error ? " error" : ""}` }, value); }

function field(label, object, name, options = {}) {
  const { type = "text", note, placeholder, optional = false, required = false, wide = false, min, max, step,
    values } = options;
  const value = object[name];
  let input;
  if (values) {
    input = element("select", {}, values.map(([value, text]) => element("option", { value }, text)));
    input.value = value ?? "";
  } else {
    input = element("input", { type, placeholder, spellcheck: "false", min, max, step, required });
    if (type === "checkbox") input.checked = Boolean(value);
    else if (type === "time") input.value = normalizeTime(value);
    else if (type === "number" && Number.isNaN(value)) {
      input.value = "";
      if (optional) object[name] = null;
    }
    else input.value = value ?? "";
  }
  input.addEventListener("input", () => {
    if (type === "checkbox") object[name] = input.checked;
    else if (type === "number") {
      if (input.validity?.badInput) object[name] = Number.NaN;
      else if (input.value === "" && optional) object[name] = null;
      else object[name] = Number.isFinite(input.valueAsNumber) ? input.valueAsNumber : Number.NaN;
    }
    else if (input.value === "" && optional) object[name] = null;
    else object[name] = input.value.trim();
  });
  return element("label", { class: type === "checkbox" ? "check" : wide ? "wide" : "" },
    type === "checkbox" ? [input, label] : [label, input, note && element("span", { class: "field-note" }, note)]);
}

function number(label, object, name, options = {}) {
  return field(label, object, name, { type: "number", min: 0, step: 1, ...options });
}
function port(label, object, name, optional = false) {
  return number(label, object, name, { min: 1, max: 65535, optional });
}
function check(label, object, name) { return field(label, object, name, { type: "checkbox" }); }

function timeZoneField(label, config, name) {
  const current = config[name];
  const usesPreset = commonTimeZones.some(([zone]) => zone === current);
  const selected = { zone: usesPreset ? current : "custom" };
  const manual = { zone: usesPreset ? "" : current };
  const picker = field(label, selected, "zone", {
    values: [...commonTimeZones, ["custom", "自定义 IANA 时区"]],
  });
  const custom = field(`自定义${label}（IANA）`, manual, "zone", { placeholder: "例如 America/Phoenix" });
  custom.hidden = usesPreset;
  picker.addEventListener("input", () => {
    const useCustom = selected.zone === "custom";
    custom.hidden = !useCustom;
    config[name] = useCustom ? manual.zone : selected.zone;
  });
  custom.addEventListener("input", () => { config[name] = manual.zone; });
  return element("div", { class: "time-zone-field" }, picker, custom);
}

function secret(label, type = "password") {
  const input = element("input", { type, autocomplete: "off", spellcheck: "false" });
  return [element("label", {}, label, input), input];
}

function heading(title, description, control) {
  return element("div", { class: "page-heading" },
    element("div", {}, element("h1", {}, title), element("p", {}, description)), control);
}

function facts(entries, className = "facts") {
  return element("dl", { class: className }, entries.map(([label, value]) =>
    element("div", {}, element("dt", {}, label), element("dd", {}, value instanceof Node ? value : text(value)))));
}

function table(headers, rows) {
  return element("div", { class: "scroll-table", tabindex: "0" }, element("table", {},
    element("thead", {}, element("tr", {}, headers.map((label) => element("th", { scope: "col" }, label)))),
    element("tbody", {}, rows.map((row) => element("tr", {}, row.map((cell) => element("td", {}, cell)))))));
}

function row(title, description, ...controls) {
  return element("div", { class: "action-row" },
    element("div", {}, element("h3", {}, title), description && element("p", {}, description)),
    element("div", { class: "actions inline" }, ...controls));
}

function showNotice(title, details = "", failed = false, scroll = false, { dismissible = true } = {}) {
  const notice = byId("notice");
  notice.classList.toggle("error", failed);
  const head = element("div", { class: "notice-head" }, element("strong", {}, title),
    dismissible ? element("button", {
      type: "button", class: "notice-close", "aria-label": "关闭提示",
      onclick: () => { notice.hidden = true; },
    }, "×") : null);
  notice.replaceChildren(head);
  if (details) notice.append(element("details", { class: "details", open: failed },
    element("summary", {}, "操作详情"), element("pre", {}, details)));
  notice.hidden = false;
  if (scroll) notice.scrollIntoView({ block: "nearest" });
}

function showError(error) { showNotice("操作失败", error.message, true, true); }

async function copyToClipboard(value, fallbackInput) {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      showNotice("已复制订阅链接");
      return;
    }
  } catch { /* fallback below */ }
  try {
    const temp = document.createElement("textarea");
    temp.value = value;
    temp.setAttribute("readonly", "");
    temp.style.position = "fixed";
    temp.style.opacity = "0";
    document.body.append(temp);
    temp.select();
    const ok = document.execCommand("copy");
    temp.remove();
    if (ok) {
      showNotice("已复制订阅链接");
      return;
    }
  } catch { /* fallback below */ }
  if (fallbackInput) {
    fallbackInput.focus();
    fallbackInput.select();
  }
  showNotice("已选中订阅链接，请手动复制");
}

async function api(path, payload) {
  const headers = { Authorization: `Bearer ${key}` };
  if (payload) headers["Content-Type"] = "application/json";
  const response = await fetch(path, {
    method: payload ? "POST" : "GET", headers, cache: "no-store", credentials: "omit",
    body: payload ? JSON.stringify(payload) : undefined,
  }).catch(() => { throw new Error("无法连接面板，请检查 SSH 隧道和终端中的面板进程。"); });
  const raw = await response.text();
  let data = {};
  if (raw) {
    try { data = JSON.parse(raw); } catch {
      if (!response.ok) throw new Error(`面板返回异常（HTTP ${response.status}）`);
      throw new Error("面板返回了无法解析的响应。");
    }
  }
  if (!response.ok) throw new Error(data.error ?? `HTTP ${response.status}`);
  return data;
}

function readAction(action, extra = {}) { return api("/api/action", { action, ...extra }); }

function isDraftModified(edit) {
  if (edit.ports !== "keep" || (edit.email && edit.email.trim())) return true;
  if (JSON.stringify(edit.config) !== edit.original) return true;
  if (edit.argoUi && edit.argoUiOriginal) {
    if (edit.argoUi.token.trim() !== "") return true;
    if (edit.argoUi.mode !== edit.argoUiOriginal.mode || edit.argoUi.addresses !== edit.argoUiOriginal.addresses) return true;
  }
  return false;
}

async function refresh() {
  model = await api("/api/state");
  for (const [id, edit] of drafts) {
    if (!isDraftModified(edit)) drafts.delete(id);
  }
  render();
}

function syncBusyUi() {
  byId("logout").disabled = busy;
  if (busy) byId("workspace").setAttribute("aria-busy", "true");
  else byId("workspace").removeAttribute("aria-busy");
  for (const control of byId("page").querySelectorAll("button")) {
    if (busy) {
      if (!("wasDisabled" in control.dataset)) {
        control.dataset.wasDisabled = control.disabled ? "1" : "0";
      }
      control.disabled = true;
    } else if ("wasDisabled" in control.dataset) {
      control.disabled = control.dataset.wasDisabled === "1";
      delete control.dataset.wasDisabled;
    }
  }
}

async function runJob(label, payload, clearDraft) {
  if (busy) return;
  if (panelClosed) throw new Error("面板会话已关闭");
  busy = true;
  syncBusyUi();
  showNotice(`${label}进行中…`, "", false, true, { dismissible: false });
  try {
    const { id } = await api("/api/action", payload);
    let job;
    do {
      await new Promise((resolve) => setTimeout(resolve, 650));
      job = await api(`/api/jobs/${id}`);
    } while (job.state === "running");
    if (job.state === "done" && job.closes_panel) {
      panelClosed = true;
      showNotice(`${label}完成，面板会话将关闭`, job.result.message, false, true);
      byId("page").replaceChildren(card("会话即将关闭", element("p", {}, job.result.message)));
      return;
    }
    if (job.state === "done" && clearDraft) drafts.delete(clearDraft);
    if (job.state === "done" && payload.action === "traffic-set") trafficUsageDraft = undefined;
    try { await refresh(); } catch (error) {
      showNotice(job.state === "done" ? `${label}完成，状态刷新失败` : `${label}失败`,
        [job.result.message, error.message].join("\n"), true, true);
      return;
    }
    const title = job.state === "done" ? job.result.summary ?? `${label}完成` : `${label}失败`;
    showNotice(title, job.result.message === title ? "" : job.result.message, job.state === "failed", true);
  } finally {
    busy = false;
    syncBusyUi();
  }
}

function dialog(title, content, { confirm = "确认", cancel = "取消", onlyClose = false } = {}) {
  const window = byId("dialog");
  if (window.open) return Promise.resolve(false);
  byId("dialog-title").textContent = title;
  byId("dialog-content").replaceChildren(content);
  byId("dialog-cancel").textContent = onlyClose ? "关闭" : cancel;
  byId("dialog-confirm").textContent = confirm;
  byId("dialog-confirm").hidden = onlyClose;
  window.returnValue = "";
  window.showModal();
  return new Promise((resolve) => {
    window.addEventListener("close", () => {
      byId("dialog-content").replaceChildren();
      resolve(window.returnValue === "ok");
    }, { once: true });
  });
}

async function confirmJob(title, description, payload, options = {}) {
  if (await dialog(title, element("p", {}, description), { confirm: options.label })) {
    await runJob(title, payload);
  }
}

function currentPage() {
  const hash = location.hash.slice(1);
  return pages.some(([id]) => id === hash) ? hash : "overview";
}

function render() {
  if (!model || panelClosed) return;
  const page = currentPage();
  byId("nav").replaceChildren(...pages.map(([id, label]) => element("a", {
    href: `#${id}`, "aria-current": id === page ? "page" : null,
    class: id === "maintenance" ? "divider" : "",
  }, label)));
  const renderers = {
    overview, deployment, nodes, traffic, subscriptions, tokens, domain, argo, certificate, maintenance,
  };
  byId("page").replaceChildren(renderers[page]());
  if (busy) syncBusyUi();
}

function bytes(value) {
  if (value == null) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
  const power = value === 0 ? 0 : Math.min(Math.floor(Math.log(value) / Math.log(1024)), 6);
  return `${(value / (1024 ** power)).toLocaleString("zh-CN", { maximumFractionDigits: 2 })} ${units[power]}`;
}

function time(value, zone) {
  if (!value) return "—";
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "medium", timeStyle: "short", timeZone: zone,
  }).format(value * 1000);
}

function serviceBadge(state, required = true) {
  if (state === "active") return element("span", { class: "badge on" }, "运行中");
  if (!required && (state === "inactive" || state === "dead")) return element("span", { class: "badge" }, "未启用");
  if (state === "activating" || state === "reloading") return element("span", { class: "badge wait" }, "启动中");
  return element("span", { class: required ? "badge err" : "badge" }, state === "failed" ? "异常" : "未运行");
}

function overview() {
  const snapshot = model.status?.traffic;
  const cap = snapshot?.monthly_limit_bytes;
  const used = snapshot?.total_used_bytes;
  const published = new Set(model.generation?.direct_nodes ?? []);
  const config = model.config;
  const argoSummary = config.argo.enabled
    ? `${config.argo.tunnel_domain || "已启用"}（${model.argo_preferred_ip_count ?? 0} 优选 IP）`
    : "未启用";
  const monthly = card("月度流量",
    element("div", { class: "usage-heading" },
      element("div", {}, element("strong", { class: "mono" }, bytes(used)),
        snapshot && element("span", { class: "limit" }, ` / ${cap ? bytes(cap) : "不限"}`)),
      cap ? element("span", { class: used > cap ? "error" : "muted" },
        `已用 ${(used / cap * 100).toFixed(1)}%${used > cap ? "（已超额）" : ""}`) : null),
    cap ? element("progress", { max: cap, value: Math.min(used, cap), "aria-label": "当月流量额度使用比例" }) : null,
    facts([["客户端上传", bytes(snapshot?.upload_bytes)], ["客户端下载", bytes(snapshot?.download_bytes)],
      ["下次重置", time(snapshot?.next_reset_ts, snapshot?.display_tz)]], "traffic-split"));
  const deploymentSummary = card("当前部署", facts([
    ["订阅地址", model.initialized ? (config.domain || config.public_ip) : null],
    ["订阅协议", model.initialized ? modeLabel(config) : null],
    ["Argo 隧道", model.initialized ? argoSummary : null],
    ["网卡", model.status?.interface ?? (model.initialized ? config.interface : null)],
    ["当前批次", element("code", {}, text(model.generation?.id))],
    ["版本", `v${model.version}`],
  ]));
  const argoPublished = (model.generation?.argo_files?.length ?? 0) > 0;
  const hasPendingChanges = model.initialized && (
    typeof model.pending_changes === "boolean"
      ? model.pending_changes
      : nodeTypes.some(([id]) => {
        const node = config.nodes[id];
        return node.enabled !== published.has(node.name);
      }) || config.argo.enabled !== argoPublished
  );
  const rows = nodeTypes.map(([id, label, protocol, engine]) => {
    const node = config.nodes[id];
    const state = published.has(node.name) ? (node.enabled ? "已发布" : "待停用") : node.enabled ? "待发布" : "已停用";
    return [label, element("code", {}, String(node.port)), protocol, engine,
      element("span", { class: `badge ${state === "已发布" ? "on" : state === "已停用" ? "" : "wait"}` }, state)];
  });
  const xrayNeeded = config.nodes.vless_reality_pqc.enabled || config.nodes.vless_xhttp_pqc.enabled
    || (config.argo.enabled && config.argo.ws_inbounds.some((ws) => ws.engine === "xray"));
  const sbNeeded = config.nodes.anytls_tcp.enabled || config.nodes.hysteria2_hop.enabled || config.nodes.tuic_bbr.enabled
    || (config.argo.enabled && config.argo.ws_inbounds.some((ws) => ws.engine === "sing-box"));
  const servicesCard = model.services ? card("运行服务", element("div", { class: "service-grid" },
    ...[
      ["订阅守护进程", model.services.vsm, model.initialized],
      ["Xray 内核", model.services.xray, model.initialized && xrayNeeded],
      ["sing-box 内核", model.services.singbox, model.initialized && sbNeeded],
      ["Argo 连接器", model.services.argo, model.initialized && config.argo.enabled],
      ["证书自动续期", model.services.cert_renew_timer, model.initialized && !isIp(config)],
    ].map(([name, state, required]) => element("div", { class: "service-item" },
      element("span", {}, name), serviceBadge(state, required))))) : null;
  const certAlert = model.initialized && !isIp(config) && model.cert_renewal?.status === "failed"
    ? element("div", { class: "callout callout-row error" },
      element("span", {},
        `证书自动续期失败（${time(model.cert_renewal.attempted_at, config.traffic.display_tz)}，连续失败 ${model.cert_renewal.consecutive_failures ?? 1} 次）：${model.cert_renewal.error || "未知错误"}`),
      element("div", { class: "actions inline" },
        button("立即续期", () => runJob("续期证书", { action: "certificate-renew" }),
          { primary: true, disabled: !model.live }),
        element("a", { href: "#certificate", class: "button-link" }, "证书管理")))
    : null;
  const warning = model.config_error ? callout(model.config_error, true)
    : model.status_error ? callout(model.status_error, true)
    : !model.initialized ? callout("尚未初始化。请在「部署配置」中填写 VPS 地址并部署。")
    : hasPendingChanges ? element("div", { class: "callout callout-row" },
      element("span", {}, "检测到主配置与当前已发布订阅不一致，应用后生效。"),
      button("立即应用配置", () => runJob("应用当前配置", { action: "apply" }), { primary: true })) : null;
  return element("div", {}, heading("运行概览", "查看本机流量、服务运行、订阅批次与节点发布状态。",
    button("刷新", refresh, { primary: true })),
  stack(certAlert, warning, element("div", { class: "overview" }, monthly, deploymentSummary), servicesCard,
    card("直连节点", table(["协议 / 节点类型", "配置端口", "传输", "内核", "订阅状态"], rows))));
}

function isIp(config) {
  const value = config.domain.trim();
  return !value || value.includes(":") || /^(\d{1,3}\.){3}\d{1,3}$/.test(value);
}

function modeLabel(config) { return isIp(config) ? "HTTP（IP 模式）" : "HTTPS"; }

function draft(id) {
  if (!drafts.has(id)) {
    drafts.set(id, {
      config: structuredClone(model.config), original: JSON.stringify(model.config),
      revision: model.revision, ports: "keep", email: "",
    });
  }
  return drafts.get(id);
}

function resetDraft(id) {
  drafts.delete(id);
  if (id === "traffic") trafficUsageDraft = undefined;
  render();
}

function staleDraftBanner(id) {
  const edit = drafts.get(id);
  if (!edit || edit.revision === model.revision) return null;
  return element("div", { class: "callout callout-row" },
    element("span", {}, "主配置已在其他页面或终端更新，当前表单基于旧版本。"),
    button("载入最新配置", () => resetDraft(id)));
}

function requireFinite(value, label, { integer = false, min = 0, max = Infinity } = {}) {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error(`请填写有效的「${label}」`);
  if (integer && !Number.isInteger(value)) throw new Error(`「${label}」必须为整数`);
  if (value < min || value > max) throw new Error(`「${label}」超出允许范围`);
}

function validateTrafficNumbers(trafficCfg) {
  requireFinite(trafficCfg.monthly_gb, "月额度", { min: 0, max: 1e7 });
  requireFinite(trafficCfg.reset_day, "每月重置日", { integer: true, min: 1, max: 31 });
}

function validateConfigNumbers(cfg) {
  requireFinite(cfg.server.acme_port, "ACME 端口", { integer: true, min: 1, max: 65535 });
  requireFinite(cfg.server.https_port, "HTTPS 端口", { integer: true, min: 1, max: 65535 });
  requireFinite(cfg.server.loopback_https_port, "HTTPS 回环端口", { integer: true, min: 1, max: 65535 });
  requireFinite(cfg.server.http_fallback_port, "HTTP 回退端口", { integer: true, min: 1, max: 65535 });
  if (cfg.server.extra_https_port != null) {
    requireFinite(cfg.server.extra_https_port, "附加 HTTPS 端口", { integer: true, min: 1, max: 65535 });
  }
  validateTrafficNumbers(cfg.traffic);
  for (const [id, label] of nodeTypes) {
    requireFinite(cfg.nodes[id].port, `${label} 监听端口`, { integer: true, min: 1, max: 65535 });
  }
  requireFinite(cfg.nodes.vless_reality_pqc.loopback_port, "VLESS REALITY PQC 回环端口", { integer: true, min: 1, max: 65535 });
  const hy2 = cfg.nodes.hysteria2_hop;
  requireFinite(hy2.hop_interval_sec, "跳跃间隔", { integer: true, min: 1, max: 65535 });
  requireFinite(hy2.up_mbps, "客户端上传带宽", { integer: true, min: 0, max: 4294967295 });
  requireFinite(hy2.down_mbps, "客户端下载带宽", { integer: true, min: 0, max: 4294967295 });
  requireFinite(hy2.server_up_mbps, "服务端上传带宽", { integer: true, min: 0, max: 4294967295 });
  requireFinite(hy2.server_down_mbps, "服务端下载带宽", { integer: true, min: 0, max: 4294967295 });
  requireFinite(cfg.argo.loopback_sub_port, "Argo 订阅回源端口", { integer: true, min: 1, max: 65535 });
  for (const ws of cfg.argo.ws_inbounds) {
    requireFinite(ws.port, `Argo WS 入站「${ws.name || ws.id || "未命名"}」端口`, { integer: true, min: 1, max: 65535 });
  }
}

function isValidIpv4(value) {
  const parts = value.split(".");
  if (parts.length !== 4) return false;
  return parts.every((part) => /^(0|[1-9]\d{0,2})$/.test(part) && Number(part) <= 255);
}

async function previewConfig(id, change, { config: overrideConfig, email = "", tunnelToken = "" } = {}) {
  const edit = draft(id);
  const candidate = overrideConfig ?? edit.config;
  validateConfigNumbers(candidate);
  const prepared = await readAction("preview", { config: candidate, ports: edit.ports });
  if (!overrideConfig && edit.ports !== "keep") {
    edit.config = prepared.config;
    edit.ports = "keep";
    render();
  }
  const config = prepared.config;
  const title = ({
    save: "仅保存主配置", apply: "应用配置", deploy: "部署并应用", "bind-domain": "启用 HTTPS",
    "argo-setup": "配置并启用 Argo", "argo-disable": "停用 Argo",
  })[change];
  const details = [
    ["部署模式", prepared.mode], ["地址", config.domain || config.public_ip],
    ["证书名称", prepared.tls_sni], ["网卡", config.interface],
    ["月额度", config.traffic.monthly_gb ? `${config.traffic.monthly_gb} GiB` : "不限"],
    ["Argo", config.argo.enabled ? "已启用" : "已停用"],
  ];
  const changes = Object.keys(config).filter((name) =>
    JSON.stringify(config[name]) !== JSON.stringify(model.config[name]));
  const sections = {
    profile_name: "订阅名称", domain: "订阅域名", public_ip: "公网 IP", interface: "网卡",
    ipv4_only: "IPv4 设置", latency_probe_url: "测速地址", server: "订阅监听端口",
    traffic: "流量账期", nodes: "节点设置", argo: "Argo 设置",
  };
  const ports = nodeTypes.map(([id, label]) => [label, config.nodes[id].enabled ? config.nodes[id].port : "停用"]);
  if (!await dialog(title, stack(
    change === "save" ? callout("仅写入主配置。应用后才会重新生成订阅并更新内核。")
      : isIp(config) ? callout("当前订阅使用明文 HTTP。请通过可信渠道导入订阅。") : null,
    facts(details), facts(ports),
    changes.length ? element("p", {}, `修改项 · ${changes.map((name) => sections[name] ?? name).join("、")}`) : null,
  ), { confirm: title })) return;
  await runJob(title, {
    action: "config", config, revision: edit.revision, change, email,
    tunnel_token: tunnelToken || null,
  }, id);
}

function configActions(id, buttons) {
  return actions(...buttons, button("还原表单", () => resetDraft(id)));
}

function deployment() {
  const edit = draft("deployment");
  const cfg = edit.config;
  const [emailField, email] = secret("证书账户邮箱（可选）", "email");
  email.value = edit.email;
  email.addEventListener("input", () => { edit.email = email.value; });
  const inputArea = grid(
    field("订阅名称", cfg, "profile_name"),
    field("网卡", cfg, "interface", { placeholder: "例如 eth0" }),
    field("订阅域名（IP 模式留空）", cfg, "domain", { placeholder: "sub.example.com" }),
    field("公网 IP", cfg, "public_ip", { placeholder: "IPv4 或 IPv6" }),
    field("客户端测速地址", cfg, "latency_probe_url", { wide: true }),
    field("节点端口方案", edit, "ports", { values: [["keep", "保留配置中的端口"], ["random", "随机分配"], ["defaults", "使用模板端口"]] }),
    emailField, check("客户端仅使用 IPv4", cfg, "ipv4_only"));
  const advanced = element("textarea", { class: "json-editor", "aria-label": "完整配置 JSON", spellcheck: "false" });
  const advancedArea = element("details", {}, element("summary", {}, "完整配置 JSON"), advanced,
    actions(button("读取表单到 JSON", () => { advanced.value = JSON.stringify(cfg, null, 2); }),
      button("载入 JSON 改动", async () => {
        const raw = advanced.value.trim();
        if (!raw) throw new Error("请先填写配置 JSON，或点击「读取表单到 JSON」。");
        let data;
        try {
          data = JSON.parse(raw);
        } catch {
          throw new Error("配置 JSON 语法无效，请检查引号、逗号与括号是否匹配。");
        }
        if (!data || typeof data !== "object" || Array.isArray(data)) {
          throw new Error("配置 JSON 顶层必须是对象（{ ... }）。");
        }
        const prepared = await readAction("preview", { config: data, ports: "keep" });
        edit.config = prepared.config;
        edit.ports = "keep";
        render();
        showNotice("JSON 已载入表单", "预览并应用后才会写入主配置。");
      })));
  advancedArea.addEventListener("toggle", () => {
    if (advancedArea.open && !advanced.value) advanced.value = JSON.stringify(cfg, null, 2);
  });
  return element("div", {}, heading("部署配置", "初始化或重新配置 VPS，预览后部署设备订阅和代理内核。"),
    stack(
      staleDraftBanner("deployment"),
      model.config_error && callout(`现有主配置无法读取。表单已载入默认模板。\n${model.config_error}`, true),
      card("基础配置", inputArea, actions(
        button("检测公网 IP 和网卡", async () => {
          const detected = await readAction("detect-network");
          if (detected.public_ip) cfg.public_ip = detected.public_ip;
          if (detected.interface) cfg.interface = detected.interface;
          render();
        }, { disabled: !model.live }),
        button("查看默认模板", async () => {
          const defaults = await api("/api/defaults");
          await dialog("默认配置模板", element("pre", {}, JSON.stringify(defaults, null, 2)), { onlyClose: true });
        }))),
      card("订阅监听", grid(port("ACME 端口", cfg.server, "acme_port"),
        port("HTTPS 端口", cfg.server, "https_port"), port("HTTPS 回环端口", cfg.server, "loopback_https_port"),
        port("HTTP 回退端口", cfg.server, "http_fallback_port"),
        port("附加 HTTPS 端口（可选）", cfg.server, "extra_https_port", true))),
      card("初始节点与账期",
        element("p", {}, "手动指定端口时，在基础配置中选择「保留配置中的端口」。"),
        grid(...nodeTypes.map(([id, label]) => port(`${label} 端口`, cfg.nodes[id], "port")),
          number("月额度（GiB，0 表示不限）", cfg.traffic, "monthly_gb", { step: "any" }),
          number("每月重置日", cfg.traffic, "reset_day", { min: 1, max: 31 }))),
      card("高级编辑", advancedArea),
      configActions("deployment", [
        button("预览部署", () => previewConfig("deployment", "deploy", { email: email.value }), { primary: true }),
        button("预览并仅保存", () => previewConfig("deployment", "save")),
        button("预览并应用", () => previewConfig("deployment", "apply"), { disabled: !model.initialized }),
      ])));
}

function nodes() {
  const cfg = draft("nodes").config;
  const panels = nodeTypes.map(([id, label, protocol, engine]) => {
    const node = cfg.nodes[id];
    let additional = [];
    if (id === "vless_reality_pqc") additional = [
      port("回环端口", node, "loopback_port"),
      field("REALITY SNI", node, "decoy_sni"),
      field("连接地址（可选）", node, "connect_host", { optional: true }),
    ];
    if (id === "hysteria2_hop") additional = [
      field("跳跃端口范围（可选）", node, "hop_range", { optional: true, placeholder: "例如 30000-35000" }),
      number("跳跃间隔（秒）", node, "hop_interval_sec"),
      number("客户端上传（Mbps）", node, "up_mbps"), number("客户端下载（Mbps）", node, "down_mbps"),
      number("服务端上传（Mbps）", node, "server_up_mbps"), number("服务端下载（Mbps）", node, "server_down_mbps"),
      check("启用 Salamander 混淆", node, "obfs_salamander"),
    ];
    return card(label, element("p", {}, `${engine} · ${protocol}`),
      grid(field("节点名称", node, "name"), port("监听端口", node, "port"), ...additional, check("启用节点", node, "enabled")));
  });
  const portChoice = async (ports) => {
    const edit = draft("nodes");
    const result = await readAction("preview", { config: edit.config, ports });
    edit.config = result.config;
    render();
    showNotice("端口已更新到表单", "预览并应用后生效。");
  };
  return element("div", {}, heading("节点端口", "管理五种直连节点的端口、开关和协议参数。"),
    stack(staleDraftBanner("nodes"), actions(button("恢复模板端口", () => portChoice("defaults")),
      button("随机分配端口", () => portChoice("random"))), ...panels,
    configActions("nodes", [
      button("预览并应用", () => previewConfig("nodes", "apply"), { primary: true, disabled: !model.initialized }),
    ])));
}

function normalizeTime(value) {
  const match = /^\s*(\d{1,2}):(\d{1,2})\s*$/.exec(String(value ?? ""));
  if (!match) return value ?? "00:00";
  return `${match[1].padStart(2, "0")}:${match[2].padStart(2, "0")}`;
}

function traffic() {
  const edit = draft("traffic");
  const config = edit.config.traffic;
  const revision = edit.revision;
  const hasConfig = model.initialized;
  const current = model.status?.traffic;
  const usage = trafficUsageDraft ?? { upload_gib: null, download_gib: null };
  const usageForm = element("form", {
    novalidate: true,
    oninput: () => { trafficUsageDraft = usage; },
    onsubmit: (event) => {
      event.preventDefault();
      if (busy || panelClosed) return;
      void saveTrafficUsage({ ...usage }, current).catch(showError);
    },
  }, grid(
    number("已用客户端上传（GiB）", usage, "upload_gib", {
      step: "any", optional: true,
      placeholder: current ? `当前 ${bytes(current.upload_bytes)}，留空保持不变` : "例如 1.5",
    }),
    number("已用客户端下载（GiB）", usage, "download_gib", {
      step: "any", optional: true,
      placeholder: current ? `当前 ${bytes(current.download_bytes)}，留空保持不变` : "例如 12.5",
    }),
  ),
  actions(
    element("button", { type: "submit", class: "primary", disabled: !hasConfig }, "保存已用流量"),
    button("清零本期用量", () => saveTrafficUsage({ upload_gib: 0, download_gib: 0 }, current), { disabled: !hasConfig }),
  ));
  return element("div", {}, heading("流量账期", "设置月额度、账期与时区，或校准本期账本。"),
    stack(staleDraftBanner("traffic"),
      card("账期设置", grid(
        number("月额度（GiB，0 表示不限）", config, "monthly_gb", { step: "any" }),
        number("每月重置日", config, "reset_day", { min: 1, max: 31 }),
        field("重置时间", config, "reset_time", { type: "time" }),
        timeZoneField("重置时区", config, "reset_tz"),
        timeZoneField("显示时区", config, "display_tz")),
      actions(
        button("保存账期设置", () => {
          validateTrafficNumbers(config);
          return runJob("保存账期设置", { action: "traffic-configure", traffic: config, revision }, "traffic");
        }, { primary: true, disabled: !hasConfig }),
        button("还原表单", () => resetDraft("traffic")),
      )),
      card("流量校准", element("p", {}, "填写需要校准的上传或下载流量（GiB），留空项将保持当前精确字节数不变。"),
        facts([["当前已用上传", bytes(current?.upload_bytes)], ["当前已用下载", bytes(current?.download_bytes)]]),
        usageForm),
      card("账本维护", element("details", {}, element("summary", {}, "手动记录服务器统计"),
        element("p", {}, "将服务器当前统计的用量立即写入账本。"),
        actions(button("记录当前统计", () => runJob("记录当前统计", { action: "traffic-checkpoint" }),
          { disabled: !hasConfig }))))));
}

async function saveTrafficUsage(usage, current) {
  if (usage.upload_gib == null && usage.download_gib == null) {
    throw new Error("请至少填写上传或下载流量中的一项，或点击「清零本期用量」。");
  }
  if (usage.upload_gib != null) {
    requireFinite(usage.upload_gib, "已用客户端上传（GiB）", { min: 0, max: 1e7 });
  }
  if (usage.download_gib != null) {
    requireFinite(usage.download_gib, "已用客户端下载（GiB）", { min: 0, max: 1e7 });
  }
  const title = "保存已用流量";
  const confirmed = await dialog(title, stack(element("p", {}, "以下数值将作为本期已用流量。"),
    facts([
      ["客户端上传", usage.upload_gib == null ? `保持当前（${bytes(current?.upload_bytes)}）` : `${usage.upload_gib} GiB`],
      ["客户端下载", usage.download_gib == null ? `保持当前（${bytes(current?.download_bytes)}）` : `${usage.download_gib} GiB`],
    ])),
  { confirm: "确认保存" });
  if (confirmed) {
    await runJob(title, {
      action: "traffic-set",
      upload_gib: usage.upload_gib,
      download_gib: usage.download_gib,
    });
  }
}

function subscriptions() {
  const selector = element("select", { "aria-label": "设备凭据" }, element("option", { value: "" }, "默认设备凭据"));
  const output = stack();
  let loadSeq = 0;
  const fetchLinks = async () => {
    if (!model.initialized) return;
    const seq = ++loadSeq;
    output.replaceChildren(element("p", {}, "正在加载订阅链接…"));
    try {
      const data = await readAction("subscriptions", { name: selector.value || null });
      if (seq !== loadSeq) return;
      output.replaceChildren();
      if (!data.https) output.append(callout("直连链接使用明文 HTTP。请通过可信渠道导入。"));
      const directHasSingbox = Boolean(data.links.direct.sing_box);
      const argoHasSingbox = Boolean(data.links.argo?.sing_box);
      output.append(subscriptionGroup("直连订阅", data.links.direct, data.qr.direct, directHasSingbox));
      if (data.links.argo) output.append(subscriptionGroup("Argo 订阅", data.links.argo, data.qr.argo, argoHasSingbox));
      if (busy) syncBusyUi();
    } catch (error) {
      if (seq !== loadSeq) return;
      output.replaceChildren(callout(error.message, true));
    }
  };
  const load = button("刷新链接", fetchLinks, { primary: true, disabled: !model.initialized });
  selector.addEventListener("change", () => {
    selectedSubscriptionDevice = selector.value;
    void fetchLinks();
  });
  if (model.initialized) {
    void api("/api/tokens").then((list) => {
      const custom = list.filter((entry) => !entry.default);
      selector.append(...custom.map((entry) => element("option", { value: entry.name }, entry.name)));
      if (selectedSubscriptionDevice && custom.some((entry) => entry.name === selectedSubscriptionDevice)) {
        selector.value = selectedSubscriptionDevice;
      } else {
        selectedSubscriptionDevice = "";
      }
      return fetchLinks();
    }).catch((error) => output.replaceChildren(callout(error.message, true)));
  } else {
    output.replaceChildren(callout("尚未初始化。请在「部署配置」中完成部署后再查看订阅。"));
  }
  return element("div", {}, heading("订阅链接", "按设备查看直连和 Argo 的订阅链接、看板与二维码。"),
    stack(card("选择设备", grid(element("label", {}, "设备凭据", selector)), actions(load)), output));
}

function subscriptionGroup(title, links, codes, hasSingbox = true) {
  const rows = subscriptionFormats.filter(([id]) => id !== "sing_box" || hasSingbox);
  return card(title, ...rows.map(([id, label, qr]) => {
    const input = element("input", {
      readonly: true, value: links[id], class: "mono", "aria-label": `${label} 链接`,
      onclick: () => input.select(),
    });
    const qrTitle = id === "shadowrocket" ? "Shadowrocket 扫码订阅二维码" : `${label} 订阅二维码`;
    return element("div", { class: "link-row" }, element("span", {}, label), input,
      button("复制", () => copyToClipboard(links[id], input)),
      qr ? button("二维码", async () => {
        const url = URL.createObjectURL(new Blob([codes[id]], { type: "image/svg+xml" }));
        try {
          await dialog(qrTitle, element("img", { src: url, alt: qrTitle }), { onlyClose: true });
        } finally {
          URL.revokeObjectURL(url);
        }
      }) : id === "index" ? element("a", {
        href: links[id], rel: "noopener noreferrer", target: "_blank", class: "button-link",
      }, "打开") : element("span"));
  }));
}

function tokens() {
  const list = element("p", {}, "正在读取设备凭据…");
  const add = { name: "" };
  const addForm = element("form", {
    novalidate: true,
    onsubmit: (event) => {
      event.preventDefault();
      if (!model.initialized || busy || panelClosed) return;
      if (!add.name.trim()) {
        showNotice("请输入设备名称", "", true, true);
        return;
      }
      void runJob("新增设备凭据", { action: "token-add", name: add.name }).catch(showError);
    },
  }, grid(field("设备名称", add, "name", { placeholder: "例如 手机", required: true })),
  actions(element("button", { type: "submit", class: "primary", disabled: !model.initialized }, "新增设备")));
  void api("/api/tokens").then((entries) => {
    if (!entries.length) {
      list.replaceWith(element("p", {}, "尚未生成凭据。部署时会生成默认凭据。"));
      return;
    }
    list.replaceWith(table(["设备名称", "脱敏凭据", "创建时间", "操作"], entries.map((entry) => [
      entry.default ? `${entry.name}（默认）` : entry.name, element("code", {}, entry.masked),
      time(entry.created_at, model.config.traffic.display_tz),
      element("div", { class: "cell-actions" },
        button("查看订阅", () => {
          selectedSubscriptionDevice = entry.default ? "" : entry.name;
          location.hash = "#subscriptions";
        }),
        button("吊销", () => confirmJob("吊销设备凭据", `设备「${entry.name}」将无法继续使用该订阅链接。`,
          { action: "token-revoke", name: entry.name }), { danger: true, disabled: entries.length < 2 })),
    ])));
    if (busy) syncBusyUi();
  }).catch((error) => list.replaceWith(callout(error.message, true)));
  return element("div", {}, heading("设备凭据", "管理每台设备的订阅 Token，并轮换代理节点使用的密钥。"),
    stack(card("设备列表", list),
      card("新增设备凭据", addForm),
      card("凭据与密钥轮换",
        row("默认订阅凭据", "轮换后，使用旧默认链接的设备需要重新导入订阅。",
          button("轮换默认凭据", () => confirmJob("轮换默认凭据", "旧默认订阅链接会立即失效。",
            { action: "token-rotate" }), { disabled: !model.initialized })),
        row("代理节点密钥", "轮换全部节点密钥后需要刷新客户端订阅。",
          button("轮换节点密钥", () => confirmJob("轮换全部节点密钥", "请在完成后刷新各客户端订阅。",
            { action: "keys-rotate" }), { danger: true, disabled: !model.initialized })))));
}

function domain() {
  const edit = draft("domain");
  const cfg = edit.config;
  const [emailField, email] = secret("证书账户邮箱（可选）", "email");
  email.value = edit.email;
  email.addEventListener("input", () => { edit.email = email.value; });
  return element("div", {}, heading("域名与 HTTPS", "绑定或更换订阅域名，签发或同步 TLS 证书。"),
    stack(staleDraftBanner("domain"),
      card("域名绑定", facts([["当前协议", model.initialized ? modeLabel(model.config) : null],
        ["当前域名", model.initialized ? model.config.domain : null]]),
      grid(field("已解析到 VPS 的域名", cfg, "domain", { placeholder: "sub.example.com" }), emailField),
      configActions("domain", [button("预览并启用 HTTPS", () => {
        if (isIp(cfg)) throw new Error("请输入有效域名。IP 模式可以在「部署配置」中设置。");
        return previewConfig("domain", "bind-domain", { email: email.value });
      }, { primary: true })])),
      card("当前证书", row("同步或续期", "按当前域名同步证书，或通过现有 ACME 流程续期。",
        button("同步证书", () => runJob("同步证书", { action: "certificate-sync" }),
          { disabled: !model.live || !model.initialized || isIp(model.config) }),
        button("续期证书", () => runJob("续期证书", { action: "certificate-renew" }),
          { disabled: !model.live || !model.initialized || isIp(model.config) })))));
}

function nextArgoInboundDefaults(cfg) {
  const usedIds = new Set(cfg.argo.ws_inbounds.map((ws) => ws.id));
  let idx = cfg.argo.ws_inbounds.length + 1;
  while (usedIds.has(`argo-ws-${idx}`)) idx += 1;
  const usedPorts = new Set([
    cfg.server.acme_port, cfg.server.https_port, cfg.server.loopback_https_port, cfg.server.http_fallback_port,
    cfg.server.extra_https_port, cfg.nodes.vless_reality_pqc.port, cfg.nodes.vless_reality_pqc.loopback_port,
    cfg.nodes.vless_xhttp_pqc.port, cfg.nodes.anytls_tcp.port, cfg.argo.loopback_sub_port,
    ...cfg.argo.ws_inbounds.map((ws) => ws.port),
  ].filter(Boolean));
  let port = 23382;
  while (usedPorts.has(port) && port < 65000) port += 1;
  return { id: `argo-ws-${idx}`, name: `argo-ws-${idx}`, protocol: "vless", engine: "xray", listen: "127.0.0.1", port, tls: false };
}

function argo() {
  const edit = draft("argo");
  const settings = edit.config.argo;
  if (!edit.argoUi) {
    const initialMode = settings.preferred_ips == null ? "file" : settings.preferred_ips.length ? "list" : "clear";
    const initialAddresses = settings.preferred_ips?.join(", ") ?? "";
    edit.argoUi = { mode: initialMode, addresses: initialAddresses, token: "" };
    edit.argoUiOriginal = { mode: initialMode, addresses: initialAddresses };
  }
  const ui = edit.argoUi;
  const hasSavedToken = Boolean(model.argo_token_configured);
  const [tokenField, token] = secret(
    hasSavedToken ? "Cloudflare Tunnel Token（留空沿用已保存 Token）" : "Cloudflare Tunnel Token（配置连接器时填写）",
  );
  token.value = ui.token;
  token.placeholder = hasSavedToken ? "已保存 Token，留空即可沿用" : "粘贴 Cloudflare Zero Trust Tunnel Token";
  token.addEventListener("input", () => { ui.token = token.value; });
  const addresses = element("textarea", { "aria-label": "优选公网 IPv4", placeholder: "最多 32 个公网 IPv4，用逗号或空格分隔" });
  addresses.value = ui.addresses;
  addresses.addEventListener("input", () => { ui.addresses = addresses.value; });
  const preferred = element("label", {}, "优选公网 IPv4", addresses);
  preferred.hidden = ui.mode !== "list";
  const fileField = field("优选 IP 文件", settings, "preferred_ips_file");
  fileField.hidden = ui.mode !== "file";
  const mode = field("优选 IP 来源", ui, "mode", { values: [
    ["file", "从服务器文件读取"], ["clear", "仅使用隧道域名"], ["list", "手动填写地址"],
  ] });
  mode.addEventListener("input", () => {
    preferred.hidden = ui.mode !== "list";
    fileField.hidden = ui.mode !== "file";
  });
  const applyArgo = (change) => {
    const candidate = structuredClone(edit.config);
    const target = candidate.argo;
    if (change === "argo-disable") {
      target.enabled = false;
      return previewConfig("argo", change, { config: candidate });
    }
    const clean = ui.addresses.split(/[,，\s]+/).filter(Boolean);
    if (ui.mode === "list") {
      if (clean.length === 0) throw new Error("请填写至少一个优选 IP，或改用隧道域名。");
      const invalidIp = clean.find((ip) => !isValidIpv4(ip));
      if (invalidIp) throw new Error(`Argo 优选 IP 格式无效：${invalidIp}（请输入公网 IPv4 地址）`);
    }
    if (change === "argo-setup" && !ui.token.trim() && !hasSavedToken) throw new Error("请输入 Cloudflare Tunnel Token。");
    if (change === "apply" && ui.token.trim()) {
      throw new Error("已填写新的 Tunnel Token，请点击上方「更新并重启连接器」保存新 Token，或清空输入框后再仅应用 Argo 配置。");
    }
    target.preferred_ips = ui.mode === "file" ? null : ui.mode === "clear" ? [] : clean;
    if (change === "argo-setup") target.enabled = true;
    if (target.enabled && !target.sub_domain) target.sub_domain = target.tunnel_domain;
    return previewConfig("argo", change, {
      config: candidate,
      tunnelToken: change === "argo-setup" ? ui.token.trim() : "",
    });
  };
  const inbounds = settings.ws_inbounds.map((entry, index) => card(`WebSocket 入站 · ${entry.name || entry.id || "新增"}`,
    grid(field("ID", entry, "id"), field("节点名称", entry, "name"),
      field("协议", entry, "protocol", { values: [["vless", "VLESS"], ["vmess", "VMess"]] }),
      field("内核", entry, "engine", { values: [["xray", "Xray"], ["sing-box", "sing-box"]] }),
      field("监听地址", entry, "listen"), port("监听端口", entry, "port"), check("回源使用 TLS", entry, "tls")),
    actions(button("移除入站", () => { settings.ws_inbounds.splice(index, 1); render(); }, { danger: true }))));
  const argoFacts = [
    ["当前部署", model.config.argo.enabled ? "已启用" : "未启用"],
    ["Tunnel Token", hasSavedToken ? "已保存在本机" : "未配置"],
    ["生效优选 IP", `${model.argo_preferred_ip_count ?? 0} 个`],
  ];
  if (model.services) argoFacts.push(["连接器服务", serviceBadge(model.services.argo, model.config.argo.enabled)]);
  return element("div", {}, heading("Argo 隧道", "配置 Cloudflare Tunnel、回源和优选 IP，检查本机与公网订阅。"),
    stack(staleDraftBanner("argo"),
      card("隧道配置",
        facts(argoFacts),
        grid(field("节点接入域名", settings, "tunnel_domain"),
          field("订阅域名", settings, "sub_domain", { placeholder: "留空时沿用节点接入域名" }),
          field("Argo 订阅名称", settings, "profile_name"),
          port("订阅回源端口", settings, "loopback_sub_port"), tokenField),
        configActions("argo", [
          button(model.config.argo.enabled ? "更新并重启连接器" : "配置并启用", () => applyArgo("argo-setup"),
            { primary: true, disabled: !model.initialized }),
          button("停用 Argo", () => applyArgo("argo-disable"), { danger: true, disabled: !model.config.argo.enabled }),
        ])),
      card("优选 IP 与路由",
        element("p", {}, model.config.argo.enabled
          ? "仅应用优选 IP、域名与入站改动并刷新回源（不修改 Tunnel Token）；如需更换 Token，请使用上方「更新并重启连接器」。"
          : "当前尚未启用 Argo；请填写 Tunnel Token 与域名后点击上方「配置并启用」，下方优选 IP 与入站设置会一并生效。"),
        grid(mode, fileField, preferred),
        actions(button("预览并应用 Argo 配置", () => applyArgo("apply"),
          { disabled: !model.initialized || !model.config.argo.enabled }))),
      ...inbounds,
      actions(button("增加 WebSocket 入站", () => {
        settings.ws_inbounds.push(nextArgoInboundDefaults(edit.config));
        render();
      })),
      card("连接器维护",
        row("检查订阅", "探测连接器、隧道连接数、本机回源、WS 入站与公网 Argo 订阅。",
          button("检查连接", () => runJob("检查 Argo", { action: "argo-check" }),
            { disabled: !model.live || !model.config.argo.enabled })),
        row("更新回源规则", "通过现有 Cloudflared 脚本重新生成规则。",
          button("更新回源", () => runJob("更新 Argo 回源", { action: "argo-refresh" }),
            { disabled: !model.live || !model.config.argo.enabled })))));
}

function certificate() {
  const status = element("pre", {}, model.initialized ? "正在读取证书状态…" : "请先初始化配置。");
  const canChange = model.initialized && model.live;
  const [emailField, email] = secret("证书账户邮箱（可选）", "email");
  const load = async () => {
    const result = await readAction("certificate-status");
    status.textContent = result.message;
  };
  if (model.initialized) void load().catch((error) => { status.textContent = error.message; });
  const autoRenewCard = model.initialized && !isIp(model.config)
    ? (() => {
      const timerActive = model.services?.cert_renew_timer === "active";
      const rec = model.cert_renewal;
      const renewSummary = rec
        ? (rec.status === "failed"
          ? `失败（${time(rec.attempted_at, model.config.traffic.display_tz)} · 连续 ${rec.consecutive_failures ?? 1} 次）：${rec.error || "未知错误"}`
          : `正常（${time(rec.last_success_at || rec.attempted_at, model.config.traffic.display_tz)}${rec.detail ? ` · ${rec.detail}` : ""}）`)
        : "暂无自动续期记录";
      return card("自动续期",
        rec?.status === "failed" ? callout(`最近一次续期失败：${rec.error || "未知错误"}`, true) : null,
        facts([
          ["定时器状态", model.services ? serviceBadge(model.services.cert_renew_timer, true) : "未知"],
          ["最近续期", renewSummary],
        ]),
        actions(
          button(timerActive ? "停用自动续期" : "启用自动续期",
            () => runJob(timerActive ? "停用自动续期" : "启用自动续期",
              { action: "certificate-auto-renew", enabled: !timerActive }),
            { danger: timerActive, primary: !timerActive, disabled: !canChange }),
        ));
    })()
    : null;
  const controls = isIp(model.config)
    ? row("IP 模式自签证书", "补齐自签证书，重新生成订阅并重载内核证书。",
      button("检查并补齐", () => confirmJob("补齐自签证书", "将应用配置并让内核重新加载证书。",
        { action: "certificate-self-signed" }), { disabled: !canChange }))
    : stack(grid(emailField),
      actions(button("申请或复用证书", () => runJob("申请或复用证书", { action: "certificate-issue", email: email.value }),
        { primary: true, disabled: !canChange }),
      button("续期当前证书", () => runJob("续期证书", { action: "certificate-renew" }), { disabled: !canChange }),
      button("同步已有证书", () => runJob("同步证书", { action: "certificate-sync" }), { disabled: !canChange })));
  return element("div", {}, heading("证书管理", "查看证书状态，续期、同步或补齐当前部署的 TLS 证书。",
    button("刷新状态", load, { disabled: !model.initialized })),
    stack(card("当前证书", status), autoRenewCard, card("证书操作", controls)));
}

async function uninstall() {
  const options = { keep_config: true, confirmation: "" };
  const entry = field("输入「卸载」确认", options, "confirmation");
  const input = entry.querySelector("input");
  input.required = true;
  input.pattern = "卸载";
  if (await dialog("卸载 vps-sub-meter",
    stack(element("p", {}, "此操作会停止代理与订阅服务，并移除程序和脚本。彻底卸载会先备份配置和流量账本。"),
      check("保留配置与流量账本", options, "keep_config"), entry), { confirm: "确认卸载" })) {
    await runJob("卸载", { action: "uninstall", ...options });
  }
}

function maintenance() {
  const versions = element("p", {}, "正在读取内核版本…");
  void readAction("core-versions").then((data) => versions.replaceWith(facts([
    ["Xray", data.xray ?? "未安装"], ["sing-box", data.sing_box ?? "未安装"], ["Cloudflared", data.cloudflared ?? "未安装"],
  ]))).catch((error) => versions.replaceWith(callout(error.message, true)));
  const live = model.live;
  return element("div", {}, heading("系统维护", "应用配置、维护内核与系统，或卸载当前部署。"),
    stack(card("内核与服务", versions,
      row("重新应用配置", "校验内核配置，重新发布订阅并按需重启服务。",
        button("应用当前配置", () => runJob("应用当前配置", { action: "apply" }), { primary: true, disabled: !model.initialized })),
      row("更新代理内核", "安装脚本指定的 Xray 和 sing-box 版本，并应用当前配置。",
        button("更新内核", () => confirmJob("更新代理内核", "更新期间代理服务可能短暂重启。",
          { action: "core-update" }), { disabled: !live })),
      row("内核网络参数", "应用现有 BBR、UDP 缓冲区和 MPTCP 设置。",
        button("调优网络", () => confirmJob("调优内核网络参数", "将使用当前部署的网络调优脚本。",
          { action: "network-tune" }), { disabled: !live }))),
    card("程序升级",
      row("检查升级兼容性", "检查现有部署的版本与迁移兼容性。",
        button("检查兼容性", () => runJob("检查升级兼容性", { action: "upgrade-check" }), { disabled: !live })),
      row("安装官方 Release", "通过现有升级脚本验证并安装发行包。完成后本次面板会话会关闭。",
        button("升级程序", async () => {
          const opts = { force_templates: false };
          if (await dialog("升级程序", check("用新版覆盖现有部署模板", opts, "force_templates"), { confirm: "开始升级" })) {
            await runJob("升级程序", { action: "upgrade-release", ...opts });
          }
        }, { disabled: !live }))),
    card("卸载", row("卸载当前部署", "可选择保留主配置和流量账本。",
      button("卸载", uninstall, { danger: true, disabled: !live })))));
}

byId("login-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const submit = event.currentTarget.querySelector("button");
  const input = byId("key");
  const error = byId("login-error");
  submit.disabled = true;
  error.hidden = true;
  key = input.value.trim();
  try {
    await api("/api/session");
    input.value = "";
    byId("login").hidden = true;
    byId("shell").hidden = false;
    try { await refresh(); } catch (err) { showError(err); }
  } catch (err) {
    key = "";
    error.textContent = err.message;
    error.hidden = false;
    input.select();
  } finally {
    submit.disabled = false;
  }
});

byId("logout").addEventListener("click", () => {
  key = "";
  drafts.clear();
  trafficUsageDraft = undefined;
  selectedSubscriptionDevice = "";
  model = undefined;
  byId("shell").hidden = true;
  byId("page").replaceChildren();
  byId("dialog-content").replaceChildren();
  byId("notice").hidden = true;
  byId("login").hidden = false;
  byId("key").focus();
});

window.addEventListener("hashchange", () => {
  if (!busy && !panelClosed) byId("notice").hidden = true;
  render();
  window.scrollTo(0, 0);
});
