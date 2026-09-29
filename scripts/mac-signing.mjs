// macOS 签名约定：Ruiss 的 .app 必须用固定证书签名，且 bundle id 固定。
//
// 为什么必须这样：系统隐私授权（辅助功能 / 输入监控 / 本地网络）对「未签名」应用是按
// 二进制 cdhash 记录的，重新编译一次哈希就变，授权全部失效、必须重新设置；用同一张
// 证书签名后，signature 的 designated requirement 变成
//     identifier "com.ruiss.app" and certificate leaf = H"<证书哈希>"
// 里面不含 cdhash，所以重新构建、替换安装都不会丢授权。
//
// 证书用钥匙串里的自签证书，本地自用不涉及 Apple 开发者账号和公证。
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';

export const SIGNING_IDENTITY = 'MiniCode Local Signing';
export const BUNDLE_ID = 'com.ruiss.app';

function run(cmd, args, env) {
  const result = spawnSync(cmd, args, { encoding: 'utf8', env });
  return {
    ok: result.status === 0,
    text: `${result.stdout || ''}${result.stderr || ''}`.trim(),
    error: result.error,
  };
}

/** 钥匙串里是否有这张签名证书（没有的话 bundler 会产出未签名的 .app）。 */
export function hasSigningIdentity(env) {
  const found = run('security', ['find-identity', '-v', '-p', 'codesigning'], env);
  return found.ok && found.text.includes(SIGNING_IDENTITY);
}

/**
 * 校验 .app 的签名是否满足「授权不丢」的条件。
 * 返回 { ok, problems, identifier, requirement }，problems 为空才算通过。
 */
export function verifyMacApp(appPath, env) {
  const problems = [];

  if (!fs.existsSync(appPath)) {
    return { ok: false, problems: [`找不到应用：${appPath}`], identifier: '', requirement: '' };
  }

  const verified = run('codesign', ['--verify', '--deep', '--strict', appPath], env);
  if (!verified.ok) {
    problems.push(`codesign --verify 失败：${verified.text || verified.error?.message || '未知原因'}`);
  }

  const info = run('codesign', ['-dv', '--verbose=4', appPath], env);
  const identifier = (info.text.match(/^Identifier=(.+)$/m) || [, ''])[1].trim();
  if (identifier !== BUNDLE_ID) {
    problems.push(`bundle id 不是 ${BUNDLE_ID}（实际 ${identifier || '空'}），换了 id 系统会当成另一个应用、授权要重设`);
  }

  // designated requirement 必须绑定证书；出现 cdhash 就说明是 ad-hoc 签名，重建即失效
  const given = run('codesign', ['-d', '-r-', appPath], env);
  const requirement = (given.text.match(/^designated =>.*$/m) || [''])[0].trim();
  if (!requirement) {
    problems.push('读不到 designated requirement，签名可能不完整');
  } else if (/cdhash/i.test(requirement)) {
    problems.push(`requirement 绑定了 cdhash（重新编译就会变，授权会丢）：${requirement}`);
  } else if (!/certificate leaf = H"/.test(requirement)) {
    problems.push(`requirement 没有绑定证书：${requirement}`);
  }

  return { ok: problems.length === 0, problems, identifier, requirement };
}
