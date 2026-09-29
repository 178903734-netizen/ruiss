// 打包 Windows 安装包（NSIS）：只在 Windows 上运行。
// 1. 编译产物目录固定用 D:/ruiss-target（见 AGENTS.md，不要落到 C 盘的 src-tauri/target）
// 2. 先编译生成 WebView2Loader.dll，再由 NSIS 钩子将它加入安装包。
//    不在静态配置中引用编译产物，否则首次编译时 DLL 尚不存在。
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { taskEnv, WINDOWS_TARGET_DIR } from './toolchain.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const targetDir = WINDOWS_TARGET_DIR;

if (process.platform !== 'win32') {
  console.error('[package:win] 该任务只支持 Windows');
  process.exit(1);
}

const cli = path.join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
const env = taskEnv();

function runTauri(args) {
  const result = spawnSync(process.execPath, [cli, ...args], {
    cwd: root,
    stdio: 'inherit',
    env,
  });
  if (result.error || result.status !== 0) {
    console.error(`[package:win] ${args[0]} 失败：${result.error?.message || `退出码 ${result.status}`}`);
    process.exit(result.status ?? 1);
  }
}

runTauri(['build', '--no-bundle']);

const loader = path.join(targetDir, 'release', 'WebView2Loader.dll');
if (!fs.existsSync(loader)) {
  console.error(`[package:win] 编译完成但缺少运行依赖：${loader}`);
  process.exit(1);
}
const bundleStartedAt = Date.now();
runTauri(['bundle', '--bundles', 'nsis']);

const bundleDir = path.join(targetDir, 'release', 'bundle', 'nsis');
if (!fs.existsSync(bundleDir)) {
  console.error(`[package:win] 没找到安装包目录：${bundleDir}`);
  process.exit(1);
}
const installers = fs.readdirSync(bundleDir).filter((name) =>
  name.toLowerCase().endsWith('.exe') &&
  fs.statSync(path.join(bundleDir, name)).mtimeMs >= bundleStartedAt - 1000);
if (installers.length === 0) {
  console.error(`[package:win] ${bundleDir} 下没有本次生成的 .exe 安装包`);
  process.exit(1);
}

for (const name of installers) {
  console.log(`[package:win] ${path.join(bundleDir, name)}`);
}
console.log(`[package:win] 完成，安装包在 ${bundleDir}`);
