#!/usr/bin/env node
// 虚拟 OAuth 伪造器：在【沙箱】auth_dir 里写一套本地自造的、绝不联网的登录凭证，
// 让 Claude Science 认为已登录（virtual@localhost.invalid），推理经 ANTHROPIC_BASE_URL
// 导去本项目翻译代理 → 通义千问。全程零 Anthropic 接触、零真实凭证。
//
// 逆向依据（见 CLAUDE.md 与 findings/）:
//   - 令牌文件: <auth_dir>/.oauth-tokens/<sanitized user_id>.enc  （目录里必须恰好一个 .enc）
//   - 文件内容(读路径 eH.decryptToken 用的 v2 格式):
//       "v2:" + base64( IV(12) ‖ AES-256-GCM(密文) ‖ authTag(16) )
//       derivedKey = hkdfSync("sha256", base64(OAUTH_ENCRYPTION_KEY), Buffer.alloc(0),
//                             "operon:aes-256-gcm:oauth", 32)
//       AAD = Buffer.from("v2:oauth")
//       明文 = JSON.stringify(tokenBlob)
//   - encryption.key 文件(HuO 解析, 换行分隔 KEY=VALUE):
//       ANTHROPIC_API_KEY_ENCRYPTION_KEY / OAUTH_ENCRYPTION_KEY / USER_SECRET_ENCRYPTION_KEY: base64≥16B
//       JWT_SIGNING_SECRET: 长度≥16 的字符串
//   - 过期判定 qP: token_expires_at 是 ISO 日期串；设远期即不触发联网刷新 _refreshToken。
//   - _tryManualApiKey 恒 null → 必须走 OAuth；provider 注册表只有 claude_ai。
//
// 用法: node make-virtual-oauth.mjs --auth-dir <沙箱/.claude-science> [--email virtual@localhost.invalid] [--force]
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

function realAncestor(p) {
  // 逐层向上找到最近的已存在祖先并 realpath，再把不存在的尾巴拼回，看穿符号链接
  let cur = path.resolve(p);
  const tail = [];
  while (!fs.existsSync(cur)) {
    tail.unshift(path.basename(cur));
    const parent = path.dirname(cur);
    if (parent === cur) break;
    cur = parent;
  }
  const base = fs.existsSync(cur) ? fs.realpathSync(cur) : cur;
  return tail.length ? path.join(base, ...tail) : base;
}
function assertNotSymlink(p) {
  try {
    if (fs.lstatSync(p).isSymbolicLink()) {
      console.error(`拒绝：${p} 是符号链接，绝不跟随写入。`);
      process.exit(3);
    }
  } catch (e) {
    if (e.code !== "ENOENT") throw e; // 不存在则允许（稍后新建）
  }
}
function safeWrite(filePath, data, mode) {
  assertNotSymlink(filePath);
  const tmp = path.join(path.dirname(filePath), `.tmp-${crypto.randomBytes(6).toString("hex")}`);
  const fd = fs.openSync(tmp, "wx", mode); // O_CREAT|O_EXCL
  try {
    fs.writeSync(fd, data);
  } finally {
    fs.closeSync(fd);
  }
  fs.renameSync(tmp, filePath);
  fs.chmodSync(filePath, mode);
}

function arg(name, def = undefined) {
  const i = process.argv.indexOf(name);
  return i > -1 && i + 1 < process.argv.length ? process.argv[i + 1] : def;
}
const has = (name) => process.argv.includes(name);

const authDir = arg("--auth-dir");
const email = arg("--email", "virtual@localhost.invalid");
const force = has("--force");
if (!authDir) {
  console.error("必须指定 --auth-dir <沙箱的 .claude-science 目录>");
  process.exit(2);
}

// —— 安全护栏：绝不写进真实凭证目录 ——
const resolvedAuth = realAncestor(authDir);
const realDir = realAncestor(path.join(os.homedir(), ".claude-science"));
if (resolvedAuth === realDir) {
  console.error(`拒绝：--auth-dir 指向真实凭证目录 ${realDir}。铁律禁止。`);
  process.exit(3);
}
if (!/\.sandbox\//.test(resolvedAuth) && !force) {
  console.error(`拒绝：--auth-dir (${resolvedAuth}) 不在 .sandbox/ 下。若确属沙箱可加 --force。`);
  process.exit(3);
}
if (!/localhost\.invalid$/.test(email)) {
  console.error(`拒绝：email 必须以 localhost.invalid 结尾（当前 ${email}），确保是假账号。`);
  process.exit(3);
}

// —— encryption.key：复用已存在的（保持 .enc 有效），否则新造 ——
const keyFile = path.join(resolvedAuth, "encryption.key");
const KEY_NAMES = [
  "ANTHROPIC_API_KEY_ENCRYPTION_KEY",
  "OAUTH_ENCRYPTION_KEY",
  "JWT_SIGNING_SECRET",
  "USER_SECRET_ENCRYPTION_KEY",
];
function parseKeyFile(txt) {
  const out = Object.create(null);
  for (const line of txt.split("\n")) {
    const eq = line.indexOf("=");
    if (eq <= 0) continue;
    const v = line.slice(eq + 1).trim();
    if (v) out[line.slice(0, eq).trim()] = v;
  }
  return out;
}
const b64_32 = () => crypto.randomBytes(32).toString("base64");

fs.mkdirSync(resolvedAuth, { recursive: true, mode: 0o700 });
let keys;
// 复用分支的 existsSync/readFileSync 都会跟随符号链接：若 encryption.key 是预置的
// 符号链接（哪怕指向目录），二者都会解引用到目标。必须先拒绝符号链接，再判断是否存在，
// 避免在真正读到内容（或触发 EISDIR 之类的非预期崩溃）之前没有先做安全检查。
assertNotSymlink(keyFile);
if (fs.existsSync(keyFile) && !force) {
  keys = parseKeyFile(fs.readFileSync(keyFile, "utf-8"));
  for (const k of KEY_NAMES) if (!keys[k]) keys[k] = k === "JWT_SIGNING_SECRET" ? b64_32() : b64_32();
} else {
  keys = {
    ANTHROPIC_API_KEY_ENCRYPTION_KEY: b64_32(),
    OAUTH_ENCRYPTION_KEY: b64_32(),
    JWT_SIGNING_SECRET: b64_32(), // 44 字符 base64，满足 ≥16
    USER_SECRET_ENCRYPTION_KEY: b64_32(),
  };
}
const keyBlob = KEY_NAMES.map((k) => `${k}=${keys[k]}`).join("\n") + "\n";
safeWrite(keyFile, keyBlob, 0o600);

// —— 令牌 blob（明文），字段对齐 _adapt / _tryOauthToken ——
const accountUuid = crypto.randomUUID();
const orgUuid = crypto.randomUUID();
const blob = {
  access_token: "sk-ant-virtual-" + crypto.randomBytes(24).toString("hex"), // 代理会剥离，值任意
  refresh_token: "",
  api_key: null,
  token_expires_at: "2099-01-01T00:00:00.000Z", // 远期 → qP() 判未过期 → 绝不联网刷新
  provider: "claude_ai",
  scopes: "user:inference user:file_upload user:profile user:mcp_servers user:plugins",
  email,
  account_uuid: accountUuid,
  subscription_type: "max",
  rate_limit_tier: null,
  seat_tier: null,
  org_uuid: orgUuid,
  billing_type: null,
  has_extra_usage_enabled: false,
};

// —— v2 GCM 加密（与二进制 ZtW/XtW 完全一致）——
function encryptTokenV2(plaintext, oauthKeyB64) {
  const ikm = Buffer.from(oauthKeyB64, "base64");
  const derived = Buffer.from(
    crypto.hkdfSync("sha256", ikm, Buffer.alloc(0), "operon:aes-256-gcm:oauth", 32)
  );
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv("aes-256-gcm", derived, iv, { authTagLength: 16 });
  cipher.setAAD(Buffer.from("v2:oauth"));
  const enc = Buffer.concat([cipher.update(plaintext, "utf-8"), cipher.final()]);
  const tag = cipher.getAuthTag();
  return "v2:" + Buffer.concat([iv, enc, tag]).toString("base64");
}
const encFileBody = encryptTokenV2(JSON.stringify(blob), keys.OAUTH_ENCRYPTION_KEY);

// —— 写 .oauth-tokens/<sanitized uuid>.enc；清掉其它 .enc 保证唯一 ——
const tokDir = path.join(resolvedAuth, ".oauth-tokens");
// tokDir 本身也要拒绝符号链接：resolvedAuth 已经被 realAncestor 看穿，
// 但 tokDir 若是预置的符号链接（指向沙箱外，最坏情况是真实 ~/.claude-science/.oauth-tokens），
// 后续 mkdirSync/chmodSync/readdirSync/unlink/safeWrite 都会原样跟随中间路径分量指向的目录，
// 等于把删除和写入动作打到了真实目录上。必须在 mkdirSync 前先拒绝。
assertNotSymlink(tokDir);
fs.mkdirSync(tokDir, { recursive: true, mode: 0o700 });
try { fs.chmodSync(tokDir, 0o700); } catch {}
for (const f of fs.readdirSync(tokDir)) {
  if (!f.endsWith(".enc")) continue;
  const p = path.join(tokDir, f);
  assertNotSymlink(p);
  fs.unlinkSync(p);
}
const userId = accountUuid.replace(/[^a-zA-Z0-9_-]/g, ""); // 与 YDO 净化一致
safeWrite(path.join(tokDir, `${userId}.enc`), encFileBody, 0o600);

// —— 自校验：用同样的解密逻辑读回来，确保 Science 能解开 ——
function decryptTokenV2(body, oauthKeyB64) {
  const ikm = Buffer.from(oauthKeyB64, "base64");
  const derived = Buffer.from(
    crypto.hkdfSync("sha256", ikm, Buffer.alloc(0), "operon:aes-256-gcm:oauth", 32)
  );
  const raw = Buffer.from(body.slice("v2:".length), "base64");
  const iv = raw.subarray(0, 12);
  const tag = raw.subarray(raw.length - 16);
  const ct = raw.subarray(12, raw.length - 16);
  const d = crypto.createDecipheriv("aes-256-gcm", derived, iv, { authTagLength: 16 });
  d.setAAD(Buffer.from("v2:oauth"));
  d.setAuthTag(tag);
  return Buffer.concat([d.update(ct), d.final()]).toString("utf-8");
}
const roundtrip = JSON.parse(decryptTokenV2(encFileBody, keys.OAUTH_ENCRYPTION_KEY));
if (roundtrip.email !== email) {
  console.error("自校验失败：解密回读的 email 不符");
  process.exit(4);
}

// —— active-org.json（Jb 只要求 org_uuid 是 UUID）——
safeWrite(
  path.join(resolvedAuth, "active-org.json"),
  JSON.stringify({ org_uuid: orgUuid }, null, 2) + "\n",
  0o600
);

console.log(JSON.stringify({
  ok: true,
  auth_dir: resolvedAuth,
  email,
  account_uuid: accountUuid,
  org_uuid: orgUuid,
  enc_file: path.join(tokDir, `${userId}.enc`),
  selfcheck: "decrypt roundtrip OK",
}, null, 2));
