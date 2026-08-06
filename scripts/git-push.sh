#!/bin/bash
# ============================================================
# ruiss 一键推送到 GitHub
#
# 用法（在项目根目录或任意位置）：
#   ./scripts/git-push.sh "提交说明"     # 提交并推送
#   ./scripts/git-push.sh --status       # 只看状态，不提交
#
# 说明：
#   - 自动执行 git add -A + commit + push
#   - 提交前检查：拒绝误提交 .rar/.zip 大文件（src-tauri.zip 1GB 事故教训）
#   - 换机器开工前先 git pull，改完再跑本脚本
# ============================================================

# 切到项目根目录（脚本在 scripts/ 下）
cd "$(dirname "$0")/.." || exit 1

# 只查看状态
if [ "$1" = "--status" ]; then
    echo "=== 当前改动 ==="
    git status --short
    echo "=== 最近提交 ==="
    git log --oneline -5
    echo "=== 与远程差异（领先/落后几个提交）==="
    git status -sb | head -1
    exit 0
fi

# 提交说明
MSG="${1:-自动提交}"
if [ -z "$1" ]; then
    echo "⚠ 未提供提交说明，使用默认：自动提交"
fi

echo "=== 1/4 检查待提交文件 ==="
git status --short

# 安全检查：拒绝 .rar/.zip（防止 1GB 大文件再次进历史）
if git status --short | grep -qiE '\.(rar|zip)$'; then
    echo ""
    echo "❌ 检测到 .rar/.zip 文件被跟踪，已中止推送！"
    echo "   这些文件不该进 git（曾经 1GB 的 src-tauri.zip 导致推送超时）"
    echo "   处理方法："
    echo "     git rm --cached <文件路径>   # 从 git 移除但保留本地文件"
    echo "     然后重跑本脚本"
    exit 1
fi

# 没有改动
if [ -z "$(git status --short)" ]; then
    echo "✅ 没有待提交的改动，无需推送"
    git log --oneline -1
    exit 0
fi

echo ""
echo "=== 2/4 提交 ==="
git add -A
git commit -m "$MSG" || exit 1

echo ""
echo "=== 3/4 推送到 GitHub ==="
git push || { echo "❌ push 失败，可能原因：网络不通 / 未配 SSH / 需要先 git pull"; exit 1; }

echo ""
echo "=== 4/4 完成 ==="
git log --oneline -1
echo "✅ 已推送到 GitHub（master）"
