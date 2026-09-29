// Ruiss 的任务环境：保证 Rust / Node 工具链能被找到。
//
// Windows：本机 PATH 里的 D:\cargo\bin 并不存在（那是 CARGO_HOME 的目录，rustup 没在那里放
// shim），真正的 cargo.exe 在 D:\rustup\toolchains\<toolchain>\bin；顺带补 GNU 编译工具。
// macOS：从面板、launchd 或非交互 shell 里启动时 PATH 只有 /usr/bin:/bin:/usr/sbin:/sbin，
// cargo 在 ~/.cargo/bin、node 在 /usr/local/bin（或 Homebrew），这里临时补进来。
// 已经在 PATH 上就原样返回，不改变原有行为。
import fs from 'node:fs';
import path from 'node:path';

export const WINDOWS_TARGET_DIR = 'D:/ruiss-target';

const CARGO_BIN_FILE = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
const HOME = process.env.USERPROFILE || process.env.HOME || '';
// 直接含 cargo 的目录（rustup shim 通常在这里）
const CARGO_BIN_DIRS = [
  process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, 'bin') : '',
  HOME ? path.join(HOME, '.cargo', 'bin') : '',
];
const TOOLCHAIN_ROOTS = [
  process.env.RUSTUP_HOME ? path.join(process.env.RUSTUP_HOME, 'toolchains') : '',
  'D:/rustup/toolchains',
  HOME ? path.join(HOME, '.rustup', 'toolchains') : '',
];
// macOS 上 cargo 之外的常用命令（node、Homebrew 工具）所在目录
const EXTRA_BIN_DIRS = ['/usr/local/bin', '/opt/homebrew/bin'];

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
  for (const dir of CARGO_BIN_DIRS) {
    if (dir && hasCargo(dir)) return dir;
  }
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
  } else {
    const entries = String(env.PATH || '').split(path.delimiter);
    const missing = EXTRA_BIN_DIRS.filter((dir) => fs.existsSync(dir) && !entries.includes(dir));
    if (missing.length > 0) {
      env.PATH = `${missing.join(path.delimiter)}${path.delimiter}${env.PATH || ''}`;
      console.log(`[ruiss] 本次已临时补入：${missing.join('、')}`);
    }
  }
  return env;
}
