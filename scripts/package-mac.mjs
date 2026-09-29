// 打包 macOS 应用与 dmg：只在 macOS 上运行。
//
// 产物固定在 src-tauri/target/release/bundle（{macos,dmg}），与 Windows 的 D:/ruiss-target 互不影响。
// 签名身份在 src-tauri/tauri.macos.conf.json 里固定为 MiniCode Local Signing —— 理由见 mac-signing.mjs：
// 只有用固定证书签名，重新构建后才不用重新设置系统的辅助功能 / 输入监控授权。
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { taskEnv } from './toolchain.mjs';
import { hasSigningIdentity, verifyMacApp, SIGNING_IDENTITY } from './mac-signing.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

if (process.platform !== 'darwin') {
  console.error('[package:mac] 该任务只支持 macOS');
  process.exit(1);
}

const env = taskEnv();

if (!hasSigningIdentity(env)) {
  console.error(`[package:mac] 钥匙串里没有签名证书「${SIGNING_IDENTITY}」，现在打包会得到未签名应用，`);
  console.error('  装上去每次都要重新设置辅助功能 / 输入监控授权。');
  console.error('  查看现有证书：security find-identity -v -p codesigning');
  process.exit(1);
}

const cli = path.join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
const bundleDir = path.join(root, 'src-tauri', 'target', 'release', 'bundle');
const startedAt = Date.now();

const built = spawnSync(process.execPath, [cli, 'build', '--bundles', 'dmg'], {
  cwd: root,
  stdio: 'inherit',
  env,
});
if (built.error || built.status !== 0) {
  console.error(`[package:mac] 打包失败：${built.error?.message || `退出码 ${built.status}`}`);
  process.exit(built.status ?? 1);
}

const appPath = path.join(bundleDir, 'macos', 'Ruiss.app');
const check = verifyMacApp(appPath, env);
if (!check.ok) {
  console.error('[package:mac] 打包完成但签名不合要求：');
  for (const problem of check.problems) console.error(`  - ${problem}`);
  process.exit(1);
}

const dmgDir = path.join(bundleDir, 'dmg');
const dmgName = fs
  .readdirSync(dmgDir)
  .filter((name) => name.endsWith('.dmg'))
  .map((name) => ({ name, mtimeMs: fs.statSync(path.join(dmgDir, name)).mtimeMs }))
  .filter((item) => item.mtimeMs >= startedAt - 1000)
  .sort((a, b) => b.mtimeMs - a.mtimeMs)[0]?.name;
if (!dmgName) {
  console.error(`[package:mac] ${dmgDir} 下没有本次生成的 dmg`);
  process.exit(1);
}

console.log('[package:mac] 完成');
console.log(`  应用：${appPath}`);
console.log(`  dmg ：${path.join(dmgDir, dmgName)}`);
console.log(`  签名：${SIGNING_IDENTITY}（requirement 绑定证书，替换安装后系统授权继续有效）`);
console.log(`  ${check.requirement}`);
console.log('  安装到 /Applications（替换旧版本）：npm run install:mac');
