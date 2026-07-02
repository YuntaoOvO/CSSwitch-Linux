import { test } from "node:test";
import assert from "node:assert";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const SCRIPT = path.join(import.meta.dirname, "..", "scripts", "make-virtual-oauth.mjs");

function mktmp() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "csswitch-oauth-"));
}
function run(authDir, extra = []) {
  return execFileSync("node", [SCRIPT, "--auth-dir", authDir, ...extra],
    { stdio: "pipe" });
}

test("auth-dir symlink whose realpath is outside .sandbox is rejected", () => {
  const t = mktmp();
  const outside = path.join(t, "outside");
  fs.mkdirSync(outside, { recursive: true });
  const sbParent = path.join(t, ".sandbox");
  fs.mkdirSync(sbParent, { recursive: true });
  const link = path.join(sbParent, "auth");
  fs.symlinkSync(outside, link); // .sandbox/auth -> outside
  assert.throws(() => run(link));
  assert.deepEqual(fs.readdirSync(outside), []); // target untouched
});

test("leaf encryption.key symlink is refused, target untouched", () => {
  const t = mktmp();
  const auth = path.join(t, ".sandbox", "auth");
  fs.mkdirSync(auth, { recursive: true });
  const secret = path.join(t, "secret-target");
  fs.writeFileSync(secret, "ORIGINAL");
  fs.symlinkSync(secret, path.join(auth, "encryption.key"));
  assert.throws(() => run(auth));
  assert.equal(fs.readFileSync(secret, "utf-8"), "ORIGINAL");
});

test("encryption.key symlinked to a DIRECTORY is rejected before dereference, not EISDIR", () => {
  // 区分「读之前拒绝」与「读之后才崩」：叶子符号链接指向目录时，
  // existsSync 会跟随符号链接判为存在，readFileSync 跟随符号链接读目录会抛 EISDIR
  // （未捕获，进程带着 Node 原生错误堆栈退出），stderr 里不会出现「符号链接」字样。
  // 若在 existsSync/readFileSync 之前先 assertNotSymlink(keyFile)，则会在解引用之前
  // 就以「是符号链接，绝不跟随写入」拒绝并 exit 3，stderr 命中 /符号链接/。
  const t = mktmp();
  const auth = path.join(t, ".sandbox", "auth");
  fs.mkdirSync(auth, { recursive: true });
  const dirTarget = path.join(t, "dir-target");
  fs.mkdirSync(dirTarget, { recursive: true });
  fs.symlinkSync(dirTarget, path.join(auth, "encryption.key"));
  let threw = false;
  let stderr = "";
  try {
    run(auth);
  } catch (e) {
    threw = true;
    stderr = (e.stderr || Buffer.alloc(0)).toString("utf-8");
  }
  assert.ok(threw, "expected run() to throw for a directory-symlinked encryption.key");
  assert.match(stderr, /符号链接/);
});

test("normal sandbox dir writes regular 0600 files", () => {
  const t = mktmp();
  const auth = path.join(t, ".sandbox", "auth");
  fs.mkdirSync(auth, { recursive: true });
  run(auth);
  for (const f of ["encryption.key", "active-org.json"]) {
    const st = fs.lstatSync(path.join(auth, f));
    assert.ok(!st.isSymbolicLink());
    assert.equal(st.mode & 0o777, 0o600);
  }
  const enc = fs.readdirSync(path.join(auth, ".oauth-tokens")).filter((x) => x.endsWith(".enc"));
  assert.equal(enc.length, 1);
});

test(".oauth-tokens pre-planted as symlink to outside dir is rejected, target untouched", () => {
  const t = mktmp();
  const auth = path.join(t, ".sandbox", "auth");
  fs.mkdirSync(auth, { recursive: true });
  const outside = path.join(t, "outside-tokens");
  fs.mkdirSync(outside, { recursive: true });
  const existingEnc = path.join(outside, "existing.enc");
  fs.writeFileSync(existingEnc, "ORIGINAL-ENC-CONTENTS");
  // .oauth-tokens 是预置的符号链接，指向沙箱外目录（最坏情况：真实 ~/.claude-science/.oauth-tokens）
  fs.symlinkSync(outside, path.join(auth, ".oauth-tokens"));
  assert.throws(() => run(auth));
  // 目标目录必须原封不动：既有文件内容不变，也没有新 .enc 写进来
  assert.equal(fs.readFileSync(existingEnc, "utf-8"), "ORIGINAL-ENC-CONTENTS");
  assert.deepEqual(fs.readdirSync(outside).sort(), ["existing.enc"]);
});
