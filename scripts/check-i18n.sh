#!/usr/bin/env bash
# i18n 统一门禁入口(Task 12)。
#
# 把 i18n 相关的机械检查收在一个入口,供逐任务验证与打包前收口复用:
#   1) 资源完整性:每个 owner 的 en / zh-CN key 与占位符一致(check_resources.py)。
#   2) 存量迁移收口:清单闭合(unresolved==0)、key 有资源与引用、fallback 是英文,
#      外加构建产物不得在运行期读仓库 locales(check_migration.py --assets)。
#   3) 检查器自身的回归测试(扫描样例 + 反向注入),防止门禁本身失效。
#
# 不在此处跑面板依赖门禁(check_panel_boundary.py)与日志来源门禁
# (check-log-scope.sh):它们各自独立、面向不同关注点,由调用方按需组合。
#
# 用法:bash scripts/check-i18n.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== check_resources =="
python3 scripts/i18n/check_resources.py

echo "== check_migration (--assets) =="
python3 scripts/i18n/check_migration.py --assets

echo "== i18n checker regression tests =="
# 检查脚本沿用仓库既有写法(裸 open().read()),在单测里会触发 ResourceWarning;
# 那不是门禁关注点,静音以免淹没真正的失败。
PYTHONWARNINGS=ignore::ResourceWarning \
  python3 -m unittest discover -s scripts/i18n -p 'test_*.py' -q

echo "check-i18n: ok"
