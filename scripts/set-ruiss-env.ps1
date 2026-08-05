# 持久化 Ruiss 开发环境变量（用户级）
# 用法：powershell -NoProfile -ExecutionPolicy Bypass -File scripts/set-ruiss-env.ps1
[Environment]::SetEnvironmentVariable('RUSTUP_HOME', 'D:\rustup', 'User')
[Environment]::SetEnvironmentVariable('CARGO_HOME', 'D:\cargo', 'User')
# 编译产物放 D 盘（C 盘空间不足；Mac 上不要设置此项，用默认 target 目录）
[Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', 'D:\ruiss-target', 'User')
$p = [Environment]::GetEnvironmentVariable('Path', 'User')
$p = $p.Replace('C:\Users\23764\.cargo\bin;', '')
$p = $p.Replace('C:\Users\23764\.cargo\bin', '')
if ($p -notlike '*D:\cargo\bin*') {
    $p = $p + ';D:\cargo\bin'
}
if ($p -notlike '*w64devkit*') {
    $p = $p + ';D:\w64devkit\w64devkit\bin'
}
[Environment]::SetEnvironmentVariable('Path', $p, 'User')
Write-Output 'ENV_OK'
