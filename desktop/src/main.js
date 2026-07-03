// CSSwitch 菜单栏面板前端。只调用后端 Tauri command，绝不碰任何密钥落盘逻辑。
// 后端只把 key 的【掩码】回显给这里；完整 key 永不进前端。
//
// 预览兜底：在普通浏览器里打开（没有 Tauri 后端）时，用 mockInvoke 返回假数据，
// 让界面能完整渲染、不报错。真实 app 里 window.__TAURI__ 存在，走真后端，此兜底不生效。
const PREVIEW = !window.__TAURI__;
const invoke = PREVIEW
  ? (cmd, args) => mockInvoke(cmd, args)
  : window.__TAURI__.core.invoke;

function mockInvoke(cmd, args) {
  switch (cmd) {
    case "get_relay_presets":
      return Promise.resolve({ presets: [
        { id: "relay-xiaomi", name: "小米 MiMo", base_url: "https://api.xiaomimimo.com/anthropic", base_url_editable: false, requires_model_override: true, builtin_models: ["mimo-v2.5-pro"] },
        { id: "relay-glm", name: "智谱 GLM", base_url: "https://open.bigmodel.cn/api/anthropic", base_url_editable: false, requires_model_override: false, builtin_models: ["glm-4.6", "glm-5", "glm-4.5-air"] },
        { id: "relay-siliconflow", name: "硅基流动", base_url: "https://api.siliconflow.cn", base_url_editable: false, requires_model_override: true, builtin_models: ["deepseek-ai/DeepSeek-V3", "zai-org/GLM-5.2"] },
        { id: "relay-openrouter", name: "OpenRouter", base_url: "https://openrouter.ai/api", base_url_editable: false, requires_model_override: false, builtin_models: ["anthropic/claude-sonnet-5", "anthropic/claude-opus-4.8-fast"] },
        { id: "relay-custom", name: "自定义", base_url: "", base_url_editable: true, requires_model_override: true, builtin_models: [] },
      ] });
    case "get_config":
      return Promise.resolve({ provider: "deepseek", proxy_port: 18991, sandbox_port: 8990, mode: "proxy", keys: { deepseek: "", qwen: "" }, relay: {} });
    case "fetch_relay_models":
      return Promise.resolve({ models: [ { id: "glm-4.6", supports_tools: true }, { id: "glm-5", supports_tools: null }, { id: "glm-lite", supports_tools: false } ], source: "live", error_kind: null, upstream_status: 200 });
    case "save_relay_config":
      return Promise.resolve({ committed: true, hint: "（预览模式：假装已验证保存）" });
    case "set_mode":
    case "open_official":
      return Promise.resolve(null);
    case "status":
      return Promise.resolve({ proxy: "amber", sandbox: "amber", upstream: "amber" });
    case "save_provider_key":
      return Promise.resolve("••••••••••" + String((args && args.key) || "").slice(-4));
    case "start_proxy":
      return Promise.resolve({ port: 18991 });
    case "verify_key":
      return Promise.resolve({ ok: true, hint: "（预览模式：假装 key 有效）" });
    case "one_click_login":
      return Promise.resolve({ url: "http://127.0.0.1:8990" });
    case "run_doctor":
      return Promise.resolve("（预览模式：后端未运行，这里是占位文本）");
    case "app_version":
      return Promise.resolve("0.0.0-preview");
    case "open_release_page":
    case "report_bug":
    case "open_logs":
      return Promise.resolve(null);
    default:
      return Promise.resolve(null);
  }
}

const $ = (id) => document.getElementById(id);
const els = {};
let statusTimer = null;
let busy = false;
let mode = "proxy"; // "proxy" 第三方 | "official" 官方
let presets = [];        // get_relay_presets 结果
let relayCfg = {};       // get_config().relay：{<id>:{key,base_url,model}}

const KEY_LABELS = { deepseek: "DeepSeek API Key", qwen: "DashScope (通义千问) API Key", relay: "中转站 API Key / Token" };

function setMsg(text, kind) {
  els.msg.textContent = text;
  els.msg.className = "msg" + (kind ? " " + kind : "");
}

function setLight(el, state) {
  // state: "green" | "amber" | "red"
  const cls = { green: "g", amber: "a", red: "r" }[state] || "a";
  el.className = "lt " + cls;
}

function setBusy(on) {
  busy = on;
  [els.oneClickBtn, els.stopBtn, els.saveKeyBtn, els.fetchModelsBtn, els.saveRelayBtn, els.skipVerifyBtn].forEach(
    (b) => b && (b.disabled = on)
  );
  // 松开忙碌时，saveRelayBtn 的禁用状态交回门控（requires_model_override 未选模型仍应禁用），
  // 避免 setBusy(false) 覆盖 refreshSaveGate 刚算好的门控（修 Task12 review Important）。
  if (!on) refreshSaveGate();
}

async function call(cmd, args) {
  return await invoke(cmd, args);
}

async function loadConfig() {
  try {
    presets = ((await call("get_relay_presets")) || {}).presets || [];
    fillPresetSelect();
    const cfg = await call("get_config");
    relayCfg = cfg.relay || {};
    const prov = cfg.provider || "deepseek";
    if (prov.startsWith("relay-")) {
      els.provider.value = "relay";          // 顶层哨兵
      els.relayPreset.value = prov;          // 预设下拉选具体 id
    } else {
      els.provider.value = prov;
    }
    els.proxyPort.value = cfg.proxy_port ?? 18991;
    els.sandboxPort.value = cfg.sandbox_port ?? 8990;
    window._keys = cfg.keys || {};
    applyMode(cfg.mode === "official" ? "official" : "proxy");
    reflectProvider();
    reflectPreset();
  } catch (e) {
    setMsg("读取配置失败：" + e, "err");
  }
}

// 当前选中的预设 id：顶层 provider=relay 哨兵时，取预设下拉值。
function currentPresetId() {
  return els.relayPreset.value || "relay-glm";
}
function currentPreset() {
  return presets.find((p) => p.id === currentPresetId()) || null;
}

// 铺预设下拉（一次）。
function fillPresetSelect() {
  els.relayPreset.innerHTML = "";
  for (const p of presets) {
    const o = document.createElement("option");
    o.value = p.id;
    o.textContent = p.name;
    els.relayPreset.appendChild(o);
  }
}

// 应用模式到 UI（不落盘）：切 panel class、分段高亮、hero 按钮文案。
function applyMode(m) {
  mode = m === "official" ? "official" : "proxy";
  els.panel.classList.toggle("mode-official", mode === "official");
  els.modeSeg.querySelectorAll(".seg-btn").forEach((b) =>
    b.classList.toggle("active", b.dataset.mode === mode)
  );
  els.oneClickBtn.textContent =
    mode === "official" ? "打开官方 Claude Science ↗" : "⚡ 一键开始";
  updateRelayUI();
}

// 中转站区块可见性：仅「第三方模式 且 provider=relay」时显示。用 JS 计算而非纯 CSS，
// 保证它绝不与官方模式并存（也就不必和 mode-official 抢 CSS 优先级）。
function updateRelayUI() {
  const show = mode !== "official" && els.provider.value === "relay";
  els.panel.classList.toggle("show-relay", show);
}

// 点分段切换：先落盘（切官方时后端会顺带停第三方链路），成功再翻 UI；失败保持旧模式、如实报错。
async function switchMode(m) {
  if (m === mode) return;
  setBusy(true);
  try {
    await call("set_mode", { mode: m });
  } catch (e) {
    // 失败不动 UI（旧模式仍生效），错误提示不被后续覆盖。
    setMsg("切换模式失败：" + e, "err");
    setBusy(false);
    return;
  }
  applyMode(m);
  setBusy(false);
  setMsg(
    mode === "official"
      ? "已切到官方模式：第三方代理/沙箱已停，点上方按钮打开你真实的 Claude Science。"
      : "已切到第三方模式：填 key 后点「一键开始」。"
  );
  await refreshStatus();
}

// 官方模式的主按钮：干净打开真实 Claude Science（后端用 open，不注入环境变量）。
async function openOfficial() {
  setBusy(true);
  setMsg("正在打开官方 Claude Science…");
  try {
    await call("open_official");
    setMsg("已打开官方 Claude Science（走你自己的官方登录与订阅）。", "ok");
  } catch (e) {
    setMsg("打开失败：" + e, "err");
  } finally {
    setBusy(false);
  }
}

// hero 按钮按当前模式分派。
async function heroClick() {
  if (mode === "official") {
    await openOfficial();
  } else {
    await oneClick();
  }
}

function reflectProvider() {
  const p = els.provider.value;
  els.keyLabel.textContent = KEY_LABELS[p] || "API Key";
  const masked = (window._keys && window._keys[p]) || "";
  els.keyInput.value = "";
  els.keyInput.placeholder = masked
    ? "已存：" + masked
    : p === "relay"
    ? "粘贴中转站 key / token（只存本地）"
    : "粘贴第三方 key（只存本地）";
  updateRelayUI();
  if (els.provider.value === "relay") reflectPreset();
}

function currentSettings() {
  return {
    provider: els.provider.value === "relay" ? currentPresetId() : els.provider.value,
    proxy_port: parseInt(els.proxyPort.value, 10) || 18991,
    sandbox_port: parseInt(els.sandboxPort.value, 10) || 8990,
  };
}

// 保存设置：失败会【抛出】，让调用方（起代理 / 一键登录）中止，
// 不再吞掉错误后拿旧配置继续、还误报成功（修 P1-4）。
async function persistSettings() {
  await call("set_config", { cfg: currentSettings() });
}

// 独立 UI 事件（改 provider / 端口）用的兜底版：失败只提示、不抛，避免未捕获拒绝。
async function persistSettingsSafe() {
  try {
    await persistSettings();
  } catch (e) {
    setMsg("保存设置失败：" + e, "err");
  }
}

async function saveKey() {
  const key = els.keyInput.value.trim();
  if (!key) {
    setMsg("请先粘贴 key。", "err");
    return;
  }
  setBusy(true);
  try {
    const masked = await call("save_provider_key", { provider: els.provider.value, key });
    window._keys[els.provider.value] = masked;
    reflectProvider();
    setMsg("已保存，正在启动代理并验证 key…", "ok");
    await persistSettings();
    // 存了 key 就自动起代理 + 用最小请求真验一次这把 key（不是「代理起来了」就当成功）。
    try {
      const v = await call("verify_key");
      if (v && v.ok) {
        setMsg("已保存，key 有效 ✓ 代理已就绪，点「一键开始」即可。", "ok");
      } else {
        setMsg("已保存，代理已起；但 key 未通过验证：" + ((v && v.hint) || "上游未接受") + " 可仍试「一键开始」。", "err");
      }
    } catch (ve) {
      // 代理没起来（缺依赖/端口占用），或验证请求发不出去（网络/上游不通）。
      setMsg("已保存；但未能验证 key：" + ve, "err");
    }
  } catch (e) {
    setMsg("保存失败：" + e, "err");
  } finally {
    setBusy(false);
    await refreshStatus();
  }
}

// 渲染模型下拉：requires_override=false 时首项「（透传）」；按 supports_tools 排序标注。
function renderModelSelect(models, requiresOverride, sourceLabel) {
  const sel = els.relayModel;
  sel.innerHTML = "";
  if (!requiresOverride) {
    const o = document.createElement("option");
    o.value = "";
    o.textContent = "（透传，用 Science 选择器）";
    sel.appendChild(o);
  }
  for (const m of models || []) {
    const o = document.createElement("option");
    o.value = m.id;
    const tag = m.supports_tools === true ? " ·工具✓" : m.supports_tools === false ? " ·无工具" : "";
    const src = sourceLabel ? " [" + sourceLabel + "]" : "";
    o.textContent = m.id + tag + src;
    sel.appendChild(o);
  }
  sel.onchange = refreshSaveGate;
}

// 门控：requires_model_override 且未选模型（值空）→ 禁用「保存并验证」。
function refreshSaveGate() {
  const p = currentPreset();
  const needModel = p && p.requires_model_override;
  const noModel = !els.relayModel.value;
  els.saveRelayBtn.disabled = !!(needModel && noModel);
}

// 「获取模型」：把 preset + base_url（+可能的新 key）交后端，起临时代理探 /v1/models。
async function fetchModels() {
  const p = currentPreset();
  if (!p) return;
  const base = p.base_url_editable ? els.relayBase.value.trim() : p.base_url;
  if (!base) {
    setMsg("请先填写中转站地址 base_url。", "err");
    return;
  }
  setBusy(true);
  setMsg("获取模型中：起临时代理 → 探 /v1/models…");
  try {
    const key = els.keyInput.value.trim(); // 有新 key 就带上；为空则后端沿用已存
    const r = await call("fetch_relay_models", { req: { preset: p.id, base_url: base, key } });
    const models = (r && r.models) || [];
    const kind = r && r.error_kind;
    const srcLabel = r && r.source === "live" ? "实时" : r && r.source === "builtin" ? "内置" : "未验证";
    renderModelSelect(models, p.requires_model_override, srcLabel);
    if (kind === "network") {
      setMsg("未能连上中转站验证，已铺内置模型（标「未验证」）。可仍试保存或重试。", "err");
    } else {
      setMsg("已获取 " + models.length + " 个模型（工具✓ 优先）。选一个后点「保存并验证」。", "ok");
    }
    refreshSaveGate();
  } catch (e) {
    // auth 硬失败（后端 Err）：不铺列表，如实提示。
    setMsg("获取模型失败：" + e, "err");
  } finally {
    setBusy(false);
  }
}

async function stopAll() {
  setBusy(true);
  setMsg("停止中…");
  try {
    await call("stop_all");
    setMsg("已停止代理与沙箱。", "ok");
    await refreshStatus();
  } catch (e) {
    setMsg("停止失败：" + e, "err");
  } finally {
    setBusy(false);
  }
}

async function oneClick() {
  setBusy(true);
  setMsg("一键开始：起代理 → 起沙箱 → 探活…");
  try {
    // 「粘贴 key → 直接一键开始」也要能走通：输入框里有新 key 就先存下，
    // 不强制用户先点「保存」（修 P1：oneClick 之前不读/不存输入框，导致无 key 起代理失败）。
    const key = els.keyInput.value.trim();
    if (key) {
      const masked = await call("save_provider_key", { provider: els.provider.value, key });
      window._keys[els.provider.value] = masked;
      els.keyInput.value = "";
      reflectProvider();
    }
    // 中转站模式：先把面板里未提交的 base_url/model/key 事务化落盘（复发防护），
    // 失败则中止一键（不带半套配置起链路）。
    if (els.provider.value === "relay") {
      const p = currentPreset();
      const base = p.base_url_editable ? els.relayBase.value.trim() : p.base_url;
      const model = els.relayModel.value;
      const r = await call("save_relay_config", {
        req: { preset: p.id, base_url: base, key: els.keyInput.value.trim(), model, skip_verify: false },
      });
      if (!r || !r.committed) {
        setMsg((r && r.hint) || "中转站配置未通过验证，未启动。", "err");
        setBusy(false);
        return;
      }
      els.keyInput.value = "";
      const cfg = await call("get_config");
      relayCfg = cfg.relay || {};
    }
    await persistSettings();
    const r = await call("one_click_login");
    // 透传后端据实回传的 msg（区分：已重新打开 / 已用新配置重启 / 沿用原有对话 / 已启动 /
    // 打开失败请手动打开），保证提示不谎报。后端未给 msg 时退回中性兜底。
    setMsg((r.msg || "已就绪，正在打开面板…") + "\n" + (r.url || ""), "ok");
    await refreshStatus();
  } catch (e) {
    setMsg("一键开始失败：" + e, "err");
  } finally {
    setBusy(false);
  }
}

async function openBrowser() {
  try {
    await call("open_url", {});
  } catch (e) {
    setMsg("打开浏览器失败：" + e, "err");
  }
}

async function runDoctor() {
  setMsg("自检中…");
  try {
    const out = await call("run_doctor");
    setMsg(out, out.includes("失败 0") ? "ok" : null);
  } catch (e) {
    setMsg("自检失败：" + e, "err");
  }
}

// 简单 semver 比较：a 是否比 b 新。
function isNewer(a, b) {
  const pa = String(a).split(".").map((n) => parseInt(n, 10) || 0);
  const pb = String(b).split(".").map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = pa[i] || 0, y = pb[i] || 0;
    if (x !== y) return x > y;
  }
  return false;
}

// 轻量检查更新：查 GitHub 最新 Release 版本号，有新版就提示并打开下载页（不自动装）。
async function checkUpdate() {
  setMsg("检查更新中…");
  let cur = "";
  try { cur = await call("app_version"); } catch (e) {}
  try {
    const resp = await fetch(
      "https://api.github.com/repos/SuperJJ007/CSswitch/releases/latest",
      { headers: { Accept: "application/vnd.github+json" } }
    );
    if (!resp.ok) throw new Error("HTTP " + resp.status);
    const data = await resp.json();
    const latest = (data.tag_name || "").replace(/^v/, "");
    if (!latest) throw new Error("无版本信息");
    if (isNewer(latest, cur)) {
      setMsg("发现新版本 v" + latest + "（当前 v" + cur + "）。正在打开下载页…", "ok");
      try { await call("open_release_page"); } catch (_) {}
    } else {
      setMsg("已是最新版本（v" + cur + "）。", "ok");
    }
  } catch (e) {
    setMsg("无法自动检查更新（多为网络或代理限制）。已打开 Releases 页，请手动查看。", "err");
    try { await call("open_release_page"); } catch (_) {}
  }
}

async function refreshStatus() {
  try {
    const s = await call("status");
    setLight(els.ltProxy, s.proxy);
    setLight(els.ltSandbox, s.sandbox);
    setLight(els.ltUpstream, s.upstream);
    const anyGreen = s.proxy === "green" || s.sandbox === "green";
    els.brandDot.className = "dot" + (s.proxy === "green" ? "" : " amber");
  } catch (e) {
    // 状态探测失败不打断，静默降级为黄灯。
    [els.ltProxy, els.ltSandbox, els.ltUpstream].forEach((l) => setLight(l, "amber"));
  }
}

// 切预设：填 base_url（只读/自定义可编）、key 掩码占位、模型下拉（builtin 起点）、门控。
function reflectPreset() {
  const p = currentPreset();
  if (!p) return;
  const saved = relayCfg[p.id] || {};
  // base_url：预设不可编 → 用预设值只读；自定义 → 用已存或空、可编。
  if (p.base_url_editable) {
    els.relayBase.value = saved.base_url || "";
    els.relayBase.readOnly = false;
    els.relayBaseHint.textContent = "自定义中转站根地址（自动补 /v1/messages 与 /v1/models）。";
  } else {
    els.relayBase.value = p.base_url;
    els.relayBase.readOnly = true;
    els.relayBaseHint.textContent = "预设地址已自动填好（只读）。填好上面的 Key 后点「获取模型」。";
  }
  // key 掩码占位（复用顶层 keyInput，reflectProvider 已处理；这里刷新 relay 专属占位）。
  const maskedKey = saved.key || "";
  els.keyInput.value = "";
  els.keyInput.placeholder = maskedKey ? "已存：" + maskedKey : "粘贴中转站 key / token（只存本地）";
  // 模型下拉：先用 builtin ∪ 已存 model 铺（无需触网）。
  const builtin = (p.builtin_models || []).map((id) => ({ id, supports_tools: null }));
  renderModelSelect(builtin, p.requires_model_override, "内置");
  if (saved.model) els.relayModel.value = saved.model;
  els.relayModelHint.textContent = p.requires_model_override
    ? "该中转站需选一个模型（不认 claude-* / 无 /v1/models）。"
    : "留「（透传）」则用 Science 选择器里的模型（claude-* 直传）。";
  refreshSaveGate();
}
async function saveRelay(skip) {
  const p = currentPreset();
  if (!p) return;
  const base = p.base_url_editable ? els.relayBase.value.trim() : p.base_url;
  const model = els.relayModel.value; // 空=透传（仅非 requires 预设可）
  if (p.requires_model_override && !model) {
    setMsg("该中转站需要选一个模型才能保存。", "err");
    return;
  }
  setBusy(true);
  setMsg(skip ? "跳过验证保存中…" : "保存中：起临时代理验证候选配置…");
  try {
    const key = els.keyInput.value.trim(); // 有新 key 就带上；空=后端沿用已存
    const r = await call("save_relay_config", {
      req: { preset: p.id, base_url: base, key, model, skip_verify: !!skip },
    });
    if (r && r.committed) {
      setMsg((r.hint || "已保存。") + " 点「一键开始」即可。", "ok");
      els.skipVerifyBtn.hidden = true;
      // 提交即清空 key 输入 + 刷新掩码/已存值（安全：完整 key 不留在 renderer）。
      els.keyInput.value = "";
      const cfg = await call("get_config");
      relayCfg = cfg.relay || {};
      reflectPreset();
    } else {
      setMsg(r && r.hint ? r.hint : "未保存。", "err");
      // 含糊态：露出「跳过验证保存」。
      els.skipVerifyBtn.hidden = !(r && r.can_skip);
    }
    await refreshStatus();
  } catch (e) {
    setMsg("保存失败：" + e, "err");
  } finally {
    setBusy(false);
    refreshSaveGate();
  }
}

function wire() {
  [
    "provider", "keyLabel", "keyInput", "saveKeyBtn", "proxyPort", "sandboxPort",
    "oneClickBtn", "stopBtn", "ltProxy", "ltSandbox", "ltUpstream",
    "msg", "brandDot", "openBrowserBtn", "doctorBtn", "updateBtn", "verLabel",
    "reportBtn", "logsBtn", "quitBtn", "modeSeg",
    "relayBase", "relayBaseHint", "fetchModelsBtn",
    "relayPreset", "relayModel", "relayModelHint", "saveRelayBtn", "skipVerifyBtn",
  ].forEach((id) => (els[id] = $(id)));
  els.panel = document.querySelector(".panel");

  els.modeSeg.querySelectorAll(".seg-btn").forEach((b) =>
    b.addEventListener("click", () => switchMode(b.dataset.mode))
  );

  els.provider.addEventListener("change", async () => {
    reflectProvider();
    await persistSettingsSafe();
  });
  els.proxyPort.addEventListener("change", persistSettingsSafe);
  els.sandboxPort.addEventListener("change", persistSettingsSafe);
  els.relayBase.addEventListener("change", persistSettingsSafe);
  els.fetchModelsBtn.addEventListener("click", fetchModels);
  els.saveKeyBtn.addEventListener("click", saveKey);
  els.stopBtn.addEventListener("click", stopAll);
  els.oneClickBtn.addEventListener("click", heroClick);
  els.openBrowserBtn.addEventListener("click", openBrowser);
  els.doctorBtn.addEventListener("click", runDoctor);
  els.updateBtn.addEventListener("click", checkUpdate);
  els.reportBtn.addEventListener("click", () =>
    call("report_bug").catch((e) => setMsg("打开反馈页失败：" + e, "err"))
  );
  els.logsBtn.addEventListener("click", () =>
    call("open_logs").catch((e) => setMsg("打开日志失败：" + e, "err"))
  );
  els.quitBtn.addEventListener("click", () => call("quit_app").catch(() => {}));
  els.relayPreset.addEventListener("change", reflectPreset);
  els.saveRelayBtn.addEventListener("click", () => saveRelay(false));
  els.skipVerifyBtn.addEventListener("click", () => saveRelay(true));
}

window.addEventListener("DOMContentLoaded", async () => {
  wire();
  await loadConfig();
  try { els.verLabel.textContent = "v" + (await call("app_version")); } catch (e) {}
  await refreshStatus();
  if (PREVIEW) {
    setMsg("预览模式：仅看界面，按钮不连后端（真实 app 里会连进程管家）。");
  } else {
    statusTimer = setInterval(refreshStatus, 2500);
  }
});
