"use strict";
const fs = require("node:fs/promises"),
  path = require("node:path"),
  os = require("node:os"),
  crypto = require("node:crypto"),
  { spawn } = require("node:child_process");
async function find(root) {
  for (const e of await fs.readdir(root, { withFileTypes: true })) {
    if (e.isSymbolicLink()) continue;
    const p = path.join(root, e.name);
    if (e.isFile() && e.name === "qpdf.exe") return p;
    if (e.isDirectory()) {
      const r = await find(p);
      if (r) return r;
    }
  }
  return null;
}
function password(p) {
  if (
    typeof p !== "string" ||
    p.length < 8 ||
    Buffer.byteLength(p) > 127 ||
    /[\r\n\0]/.test(p)
  )
    throw Error("密码须为8字符以上、UTF-8不超过127字节且不含换行");
  return p;
}
function invoke(executable, args) {
  if (args.some((x) => /[\r\n\0]/.test(x))) throw Error("无效参数");
  return new Promise((resolve, reject) => {
    const p = spawn(executable, ["@-"], {
      windowsHide: true,
      stdio: ["pipe", "ignore", "ignore"],
    });
    const timer = setTimeout(() => {
      p.kill();
      reject(Error("PDF处理超时，未保存"));
    }, 120000);
    p.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
    p.on("close", (code) => {
      clearTimeout(timer);
      code === 0
        ? resolve()
        : reject(Error("PDF结构检查/处理未通过（代码" + code + "），未保存"));
    });
    p.stdin.on("error", () => {});
    p.stdin.end(args.join("\n") + "\n", "utf8");
  });
}
async function transform(bytes, mode, options = {}, resourceRoot) {
  if (!["compress", "encrypt", "decrypt"].includes(mode))
    throw Error("无效PDF处理");
  if (mode === "encrypt") {
    password(options.password);
    if (options.owner !== undefined) {
      password(options.owner);
      if (options.owner === options.password)
        throw Error("所有者密码不可与打开密码相同");
    }
  }
  const executable =
    process.env.PDFMASTER_TEST_QPDF ||
    (await find(path.join(resourceRoot, "qpdf")));
  if (!executable) throw Error("未找到随包qpdf运行时");
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "pdfmaster-qpdf-"));
  try {
    const input = path.join(dir, "input.pdf"),
      output = path.join(dir, "output.pdf");
    await fs.writeFile(input, bytes, { mode: 0o600 });
    const open =
      mode === "decrypt" ? ["--password=" + password(options.password)] : [];
    await invoke(executable, [...open, "--check", input]);
    const args =
      mode === "compress"
        ? [
            "--object-streams=generate",
            "--recompress-flate",
            "--compression-level=9",
          ]
        : mode === "encrypt"
          ? [
              "--encrypt",
              "--user-password=" + options.password,
              "--owner-password=" +
                (options.owner || crypto.randomBytes(48).toString("hex")),
              "--bits=256",
              "--",
            ]
          : ["--decrypt"];
    await invoke(executable, [input, ...open, ...args, output]);
    await invoke(executable, [
      ...(mode === "encrypt" ? ["--password=" + options.password] : []),
      "--check",
      output,
    ]);
    const result = await fs.readFile(output);
    if (!result.length || result.length > 256 * 1024 * 1024)
      throw Error("输出大小异常");
    return mode === "compress" && result.length >= bytes.length
      ? Buffer.from(bytes)
      : result;
  } finally {
    await fs.rm(dir, { recursive: true, force: true });
  }
}
module.exports = { transform, password };
