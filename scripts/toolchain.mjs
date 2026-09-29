// Ruiss 的任务环境：保证 Rust 工具链能被找到。
//
// 本机 PATH 里的 D:\cargo\bin 并不存在（那是 CARGO_HOME 的目录，rustup 没在那里放 shim），
// 真正的 cargo.exe 在 D:\rustup\toolchains\<toolchain>\bin。cargo 不在 PATH 上时，
// 这里把工具链目录补进 PATH；已经在 PATH 上就原样返回，不影响 Mac/Linux。
import fs from 'node:fs';
import path from 'node:path';

export const WINDOWS_TARGET_DIR = 'D:/ruiss-target';

const CARGO_BIN_FILE = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
const TOOLCHAIN_ROOTS = [
  process.env.RUSTUP_HOME ? path.join(process.env.RUSTUP_HOME, 'toolchains') : '',
  'D:/rustup/toolchains',
  path.join(process.env.USERPROFILE || process.env.HOME || '', '.rustup', 'toolchains'),
];

function hasCargo(dir) {
  try {
    return fs.existsSync(path.join(dir, CARGO_BIN_FILE));
  } catch {
    return false;
  }
}

/** 找到工具链 bin 目录；PATH 上已有 cargo 时返回空串。 */
export function findCargoBin(searchPath = process.env.PATH || '') {
  const entries = String(searchPath).split(path.delimiter);
  if (entries.some((dir) => dir && hasCargo(dir))) return '';
  for (const root of TOOLCHAIN_ROOTS) {
    if (!root || !fs.existsSync(root)) continue;
    for (const name of fs.readdirSync(root)) {
      const bin = path.join(root, name, 'bin');
      if (hasCargo(bin)) return bin;
    }
  }
  return '';
}

/** 生成子进程环境变量：在 process.env 基础上补 PATH（必要时）与额外变量。 */
export function taskEnv(extra = {}) {
  const env = { ...process.env, ...extra };
  // Windows 环境变量不区分大小写；避免同时传 Path 和 PATH，导致补入的路径被丢弃。
  if (process.platform === 'win32') {
    const pathKeys = Object.keys(env).filter((key) => key.toLowerCase() === 'path');
    const searchPath = env.PATH ?? env[pathKeys[0]] ?? '';
    for (const key of pathKeys) delete env[key];
    env.PATH = searchPath;
    env.CARGO_TARGET_DIR = WINDOWS_TARGET_DIR;
  }
  const bin = findCargoBin(env.PATH);
  if (bin) {
    env.PATH = `${bin}${path.delimiter}${env.PATH || ''}`;
    console.log(`[ruiss] cargo 不在 PATH 上，本次已临时补入：${bin}`);
  }
  if (process.platform === 'win32') {
    const utilities = ['gcc.exe', 'dlltool.exe', 'windres.exe'];
    const entries = String(env.PATH || '').split(path.delimiter);
    const hasUtilities = utilities.every((file) => entries.some((dir) => dir && fs.existsSync(path.join(dir, file))));
    if (!hasUtilities) {
      const gnuBin = ['D:/w64devkit/w64devkit/bin', 'D:/w64devkit/bin']
        .find((dir) => utilities.every((file) => fs.existsSync(path.join(dir, file))));
      if (gnuBin) {
        env.PATH = `${gnuBin}${path.delimiter}${env.PATH || ''}`;
        console.log(`[ruiss] 本次已临时补入 GNU 编译工具：${gnuBin}`);
      }
    }
  }
  return env;
}
