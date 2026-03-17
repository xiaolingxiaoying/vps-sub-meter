#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# VPS 流量统计与订阅管理 自动配置脚本
# 用法:
#   bash <(curl -fsSL https://raw.githubusercontent.com/xiaolingxiaoying/vps-sub-meter/main/auto_setup.sh)
#
# 功能:
#   - 通过 vnstat + sysfs 实时监控 VPS 出口流量
#   - Python HTTP 服务下发带 subscription-userinfo 的订阅 (YAML + JSON)
#   - 同时支持 Clash Meta (YAML) 和 sing-box (JSON) 订阅格式
#   - Caddy 反向代理提供 HTTPS + Basic Auth 鉴权
#   - 支持 ?token= 参数免密访问 (给 CMFA 等不支持 BasicAuth 的客户端)
#   - 每月自动重置流量基线
#   - 每 5 分钟同步上游订阅配置
# ==============================================================================

# 配置文件路径
CONFIG_FILE="/etc/sub-srv/config.conf"

# 0. 确保交互式输入可用 (兼容 bash <(curl ...) 方式)
if [ ! -t 0 ]; then
    exec < /dev/tty
fi

# 1. 检查 root 权限
if [ "$(id -u)" != "0" ]; then
    echo "错误: 请使用 root 权限运行此脚本 (例如: sudo bash <(curl -fsSL URL))"
    exit 1
fi

# 2. 检查系统兼容性
if ! command -v apt &>/dev/null; then
    echo "错误: 此脚本仅支持 Debian/Ubuntu 系统 (需要 apt 包管理器)"
    exit 1
fi

# ===================== 输入验证函数 =====================

# 验证域名格式 (支持 IDN 和常见域名)
validate_domain() {
    local domain="$1"
    # 基本格式检查：至少有一个点，不包含非法字符
    if [[ ! "$domain" =~ ^[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?)*$ ]]; then
        return 1
    fi
    # 长度检查
    if [ ${#domain} -gt 253 ]; then
        return 1
    fi
    return 0
}

# 验证用户名 (BasicAuth 用户名限制)
validate_username() {
    local username="$1"
    # BasicAuth 用户名不能包含冒号，且不应包含空格和特殊字符
    if [[ "$username" =~ [:[:space:]] ]]; then
        return 1
    fi
    if [ ${#username} -gt 64 ]; then
        return 1
    fi
    return 0
}

# 验证流量上限 (必须是有效数字)
validate_traffic_limit() {
    local limit="$1"
    if [ -z "$limit" ]; then
        return 0  # 空值表示无限，允许
    fi
    # 必须是非负整数或浮点数
    if [[ ! "$limit" =~ ^[0-9]+\.?[0-9]*$ ]]; then
        return 1
    fi
    return 0
}

# 检查端口是否被占用
check_port_available() {
    local port="$1"
    # 使用更精确的正则表达式避免误匹配 (例如 2080 匹配到 12080)
    if ss -tuln 2>/dev/null | grep -qE "[:.]${port}[[:space:]]" || \
       netstat -tuln 2>/dev/null | grep -qE "[:.]${port}[[:space:]]"; then
        return 1
    fi
    return 0
}

# 保存配置到文件
save_config() {
    local config_dir
    config_dir=$(dirname "$CONFIG_FILE")
    mkdir -p "$config_dir"
    chmod 700 "$config_dir"

    # 使用 python3 写入配置文件，彻底避免 shell 对 $ 的展开
    # (bcrypt 哈希含 $2a$14$ 等，直接用 heredoc 或 echo 会被 shell 破坏)
    python3 - \
        "$CONFIG_FILE" \
        "${DOMAIN}" \
        "${CADDY_USER}" \
        "${PASSWORD_HASH:-}" \
        "${TRAFFIC_LIMIT_GIB}" \
        "${TZ_NAME}" \
        "${RESET_MODE:-natural_month}" \
        "${RESET_ANCHOR_DATE:-}" \
        "${RESET_HOUR:-0}" \
        "${RESET_MINUTE:-0}" \
        "${IFACE}" \
        "${TOKEN}" \
        "${BACKEND_PORT:-2080}" \
        "${USED_TRAFFIC_GIB:-0}" \
        <<'PYEOF'
import sys, os
from datetime import datetime

cfg_file   = sys.argv[1]
domain     = sys.argv[2]
caddy_user = sys.argv[3]
pass_hash  = sys.argv[4]
limit_gib  = sys.argv[5]
tz_name    = sys.argv[6]
reset_mode = sys.argv[7]
reset_anchor_date = sys.argv[8]
reset_hour = sys.argv[9]
reset_minute = sys.argv[10]
iface      = sys.argv[11]
token      = sys.argv[12]
backend_port = sys.argv[13]
used_gib   = sys.argv[14]

lines = [
    "# VPS 订阅服务配置文件",
    f"# 生成时间: {datetime.now().astimezone().isoformat()}",
    "",
    f"DOMAIN={domain!r}",
    f"CADDY_USER={caddy_user!r}",
    f"CADDY_PASS_HASH={pass_hash!r}",
    f"TRAFFIC_LIMIT_GIB={limit_gib!r}",
    f"USED_TRAFFIC_GIB={used_gib!r}",
    f"TZ_NAME={tz_name!r}",
    f"RESET_MODE={reset_mode!r}",
    f"RESET_ANCHOR_DATE={reset_anchor_date!r}",
    f"RESET_HOUR={reset_hour!r}",
    f"RESET_MINUTE={reset_minute!r}",
    f"IFACE={iface!r}",
    f"TOKEN={token!r}",
    f"BACKEND_PORT={backend_port!r}",
]
with open(cfg_file, "w", encoding="utf-8") as f:
    f.write("\n".join(lines) + "\n")
os.chmod(cfg_file, 0o600)
PYEOF
    echo "=> 配置已保存到 $CONFIG_FILE"
}

# 加载已有配置
load_config() {
    if [ -f "$CONFIG_FILE" ]; then
        echo "检测到已有配置文件: $CONFIG_FILE"
        read -rp "是否加载已有配置? [Y/n]: " load_choice
        load_choice=${load_choice:-Y}
        if [[ "$load_choice" =~ ^[Yy] ]]; then
            # 使用 python3 解析配置文件，避免 source 时 shell 展开 bcrypt 哈希中的 $
            local parse_result
            parse_result=$(python3 - "$CONFIG_FILE" <<'PYEOF'
import sys, shlex

cfg = sys.argv[1]
out = []
try:
    with open(cfg, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if "=" not in line:
                continue
            key, _, val = line.partition("=")
            key = key.strip()
            # shlex.split 处理单引号/双引号包裹的值
            try:
                val = shlex.split(val.strip())[0]
            except Exception:
                val = val.strip().strip("'\"")
            out.append(f"{key}={shlex.quote(val)}")
except Exception as e:
    print(f"ERROR: {e}", file=sys.stderr)
    sys.exit(1)

print("\n".join(out))
PYEOF
            ) || {
                echo "警告: 配置文件损坏，将使用默认配置"
                return 1
            }

            # 将 python 输出的安全赋值语句 eval 到当前 shell
            # 此时值已经被 shlex.quote 处理，不含裸 $ 展开风险
            set +u
            eval "$parse_result"
            set -u

            # 将配置文件中的字段映射到脚本使用的变量名
            CADDY_PASS_HASH="${CADDY_PASS_HASH:-}"
            PASSWORD_HASH="${CADDY_PASS_HASH:-}"
            TZ_NAME="${TZ_NAME:-America/Los_Angeles}"
            RESET_MODE="${RESET_MODE:-natural_month}"
            RESET_ANCHOR_DATE="${RESET_ANCHOR_DATE:-}"
            RESET_HOUR="${RESET_HOUR:-0}"
            RESET_MINUTE="${RESET_MINUTE:-0}"
            USED_TRAFFIC_GIB="${USED_TRAFFIC_GIB:-}"

            echo "=> 已加载配置: 域名=${DOMAIN:-未设置}, 用户=${CADDY_USER:-未设置}, 时区=${TZ_NAME:-未设置}, 网卡=${IFACE:-未设置}"
            return 0
        fi
    fi
    return 1
}

echo "=================================================="
echo "      VPS 流量统计与订阅管理 - 自动配置向导       "
echo "=================================================="

# 尝试加载已有配置
if load_config; then
    echo "(已加载配置，如需修改请直接输入新值，留空保持原值)"
else
    echo "(如果已运行过此脚本，再次运行将覆盖旧配置)"
fi
echo ""

# 3. 交互式收集配置信息
# 域名输入与验证
while true; do
    if [ -n "${DOMAIN:-}" ]; then
        read -rp "请输入绑定的域名 [当前: $DOMAIN]: " input_domain
        if [ -z "$input_domain" ]; then
            break  # 保持原值
        fi
        DOMAIN="$input_domain"
    else
        read -rp "请输入绑定的域名 (例如: sub.example.com): " DOMAIN
    fi

    if [ -z "$DOMAIN" ]; then
        echo "错误: 域名不能为空"
        continue
    fi

    if ! validate_domain "$DOMAIN"; then
        echo "错误: 域名格式无效 '$DOMAIN'"
        echo "       请使用类似 sub.example.com 的格式"
        DOMAIN=""
        continue
    fi
    break
done

# 用户名输入与验证
while true; do
    if [ -n "${CADDY_USER:-}" ]; then
        read -rp "请输入访问用户名 (用于 BasicAuth) [当前: $CADDY_USER]: " input_user
        if [ -z "$input_user" ]; then
            break  # 保持原值
        fi
        CADDY_USER="$input_user"
    else
        read -rp "请输入访问用户名 (用于 BasicAuth): " CADDY_USER
    fi

    if [ -z "$CADDY_USER" ]; then
        echo "错误: 用户名不能为空"
        continue
    fi

    if ! validate_username "$CADDY_USER"; then
        echo "错误: 用户名包含非法字符 (不能包含冒号或空格)"
        echo "       请使用字母、数字、下划线或短横线"
        CADDY_USER=""
        continue
    fi
    break
done

# URL 编码函数 (安全传递任意字符到 python3，使用 stdin 避免引号/特殊字符问题)
urlencode() {
    printf '%s' "$1" | python3 -c "import sys, urllib.parse; print(urllib.parse.quote(sys.stdin.buffer.read().decode(), safe=''), end='')"
}

# 密码输入与验证
NEED_NEW_PASSWORD=true
SAVED_PASSWORD_MODE=false
if [ -n "${CADDY_PASS_HASH:-}" ]; then
    # 检查哈希是否是合法的 bcrypt 格式 ($2a$ / $2b$ / $2y$ 开头)
    # 旧版脚本保存时 $ 会被 shell 破坏，需要检测并提示重新设置
    if [[ "${CADDY_PASS_HASH}" =~ ^\$2[aby]\$ ]]; then
        echo "检测到已保存的密码 (哈希值: ${CADDY_PASS_HASH:0:20}...)"
        read -rp "是否使用已保存的密码? [Y/n]: " use_saved
        use_saved=${use_saved:-Y}
        if [[ "$use_saved" =~ ^[Yy] ]]; then
            NEED_NEW_PASSWORD=false
            PASSWORD_HASH="$CADDY_PASS_HASH"
            echo "=> 将使用已保存的密码哈希进行 BasicAuth 认证"
            echo ""
            echo "提示: 如需在部署完成后生成含密码的一键导入链接和二维码，"
            echo "      请输入您的密码明文 (仅用于生成链接，不会额外存储)。"
            read -rs -p "请输入密码明文 (留空则跳过，链接/二维码中将显示占位符): " CADDY_PASS
            echo
            if [ -n "$CADDY_PASS" ]; then
                SAVED_PASSWORD_MODE=false
                echo "=> 已记录密码，部署完成后将生成完整的一键导入链接和二维码"
            else
                CADDY_PASS="<已保存的密码>"
                SAVED_PASSWORD_MODE=true
                echo "=> 已跳过，一键导入链接和二维码中将显示占位符 <密码>"
            fi
        fi
    else
        echo "警告: 已保存的密码哈希格式无效 (可能由旧版脚本的 bug 导致损坏)"
        echo "      请重新输入密码以修复此问题"
        CADDY_PASS_HASH=""
    fi
fi

if [ "$NEED_NEW_PASSWORD" = true ]; then
    while true; do
        read -rs -p "请输入访问密码 (用于 BasicAuth，支持特殊字符): " CADDY_PASS
        echo
        if [ -z "$CADDY_PASS" ]; then
            echo "错误: 密码不能为空，请重新输入"
            continue
        fi
        # 检查密码中是否含 URL 不安全字符，提示但不阻止
        if [[ "$CADDY_PASS" =~ [@:/] ]]; then
            echo "提示: 密码中包含特殊字符，一键导入链接将自动进行 URL 编码处理。"
        fi
        break
    done
fi

# 流量上限输入与验证
while true; do
    if [ -n "${TRAFFIC_LIMIT_GIB:-}" ]; then
        read -rp "请输入每月流量上限 (GiB，0 或留空表示无限) [当前: $TRAFFIC_LIMIT_GIB]: " input_limit
        if [ -z "$input_limit" ]; then
            break  # 保持原值
        fi
        TRAFFIC_LIMIT_GIB="$input_limit"
    else
        read -rp "请输入每月流量上限 (GiB，0 或留空表示无限，默认 0): " TRAFFIC_LIMIT_GIB
    fi

    TRAFFIC_LIMIT_GIB=${TRAFFIC_LIMIT_GIB:-0}

    if ! validate_traffic_limit "$TRAFFIC_LIMIT_GIB"; then
        echo "错误: 流量上限必须是有效的数字"
        TRAFFIC_LIMIT_GIB=""
        continue
    fi
    break
done

# 已使用流量设置 (用于在重新部署或初始化时设定起始已用量)
# 尝试从现有状态文件读取当前已用流量，供用户参考
CURRENT_USED_GIB=""
STATE_FILE_TMP="/var/lib/subsrv/tx_state.json"
if [ -f "$STATE_FILE_TMP" ] && [ -s "$STATE_FILE_TMP" ]; then
    CURRENT_USED_GIB=$(python3 - "$STATE_FILE_TMP" "${IFACE:-}" <<'PYEOF'
import json, sys, os

state_path = sys.argv[1]
iface = sys.argv[2] if len(sys.argv) > 2 else ""

try:
    with open(state_path, encoding="utf-8") as f:
        st = json.load(f)
    base_tx = st.get("base_tx")
    if base_tx is None:
        sys.exit(0)
    # 尝试读取当前 tx_bytes
    cur_tx = None
    if iface:
        try:
            with open(f"/sys/class/net/{iface}/statistics/tx_bytes", encoding="utf-8") as f:
                cur_tx = int(f.read().strip())
        except Exception:
            pass
    if cur_tx is not None:
        used = max(0, cur_tx - int(base_tx))
        used_gib = used / (1024 ** 3)
        print(f"{used_gib:.3f}")
    else:
        # 无法读取当前值，显示 0
        print("0.000")
except Exception:
    pass
PYEOF
    ) 2>/dev/null || CURRENT_USED_GIB=""
fi

while true; do
    if [ -n "${USED_TRAFFIC_GIB:-}" ]; then
        # 同时显示配置文件中保存的值和实际测量值（如果有）
        if [ -n "$CURRENT_USED_GIB" ]; then
            read -rp "请输入本周期已使用的流量 (GiB，0 表示从零开始) [当前实测: ${CURRENT_USED_GIB} GiB，配置保存值: ${USED_TRAFFIC_GIB}，留空使用实测值]: " input_used
        else
            read -rp "请输入本周期已使用的流量 (GiB，0 表示从零开始) [配置保存值: ${USED_TRAFFIC_GIB}，留空保持原值]: " input_used
        fi
        if [ -z "$input_used" ]; then
            # 留空：如果有实测值优先用实测值，否则保持配置文件中的值
            if [ -n "$CURRENT_USED_GIB" ]; then
                USED_TRAFFIC_GIB="$CURRENT_USED_GIB"
                echo "=> 使用实测已用流量: ${USED_TRAFFIC_GIB} GiB"
            fi
            break
        fi
        USED_TRAFFIC_GIB="$input_used"
    else
        if [ -n "$CURRENT_USED_GIB" ]; then
            read -rp "请输入本周期已使用的流量 (GiB，0 表示从零开始) [当前实测: ${CURRENT_USED_GIB} GiB，留空使用实测值]: " input_used
        else
            read -rp "请输入本周期已使用的流量 (GiB，0 表示从零开始，默认 0): " input_used
        fi
        if [ -z "$input_used" ]; then
            if [ -n "$CURRENT_USED_GIB" ]; then
                USED_TRAFFIC_GIB="$CURRENT_USED_GIB"
                echo "=> 使用实测已用流量: ${USED_TRAFFIC_GIB} GiB"
            else
                USED_TRAFFIC_GIB="0"
            fi
            break
        fi
        USED_TRAFFIC_GIB="$input_used"
    fi

    if ! validate_traffic_limit "$USED_TRAFFIC_GIB"; then
        echo "错误: 已使用流量必须是有效的非负数字"
        USED_TRAFFIC_GIB=""
        continue
    fi
    break
done

# 时区选择与验证
TZ_NAME="${TZ_NAME:-America/Los_Angeles}"
while true; do
    echo "请选择流量刷新/重置时区:"
    echo "  1) America/Los_Angeles (默认)"
    echo "  2) Asia/Shanghai"
    echo "  3) 自定义输入"

    if [ -n "${TZ_NAME:-}" ]; then
        read -rp "请输入选项 [当前: $TZ_NAME，回车保持当前]: " tz_choice
        if [ -z "$tz_choice" ]; then
            tz_choice=0
        fi
    else
        read -rp "请输入选项 [默认: 1]: " tz_choice
        tz_choice=${tz_choice:-1}
    fi

    case "$tz_choice" in
        0)
            ;;
        1)
            TZ_NAME="America/Los_Angeles"
            ;;
        2)
            TZ_NAME="Asia/Shanghai"
            ;;
        3)
            read -rp "请输入自定义时区 (例如: America/Los_Angeles): " custom_tz
            if [ -z "$custom_tz" ]; then
                echo "错误: 自定义时区不能为空"
                continue
            fi
            TZ_NAME="$custom_tz"
            ;;
        *)
            echo "错误: 无效选项，请输入 1/2/3"
            continue
            ;;
    esac

    if [ ! -f "/usr/share/zoneinfo/$TZ_NAME" ]; then
        echo "错误: 无效的时区 '$TZ_NAME'，请使用类似 America/Los_Angeles 的格式"
        echo "       可用时区列表: ls /usr/share/zoneinfo/"
        continue
    fi
    break
done

# 选择刷新规则 (按所选时区解释)
RESET_MODE="${RESET_MODE:-natural_month}"
RESET_ANCHOR_DATE="${RESET_ANCHOR_DATE:-}"
RESET_HOUR="${RESET_HOUR:-0}"
RESET_MINUTE="${RESET_MINUTE:-0}"

while true; do
    echo "请选择流量到期/刷新规则 (时区: $TZ_NAME):"
    echo "  1) 自然月: 每月 1 日 00:00 (默认)"
    echo "  2) 指定首个重置年月日和时间，之后每月同日同时间"
    echo "  3) 指定固定到期日 (不循环，到达后直接停机失效)"
    if [ "$RESET_MODE" = "anchored_monthly" ] && [ -n "$RESET_ANCHOR_DATE" ]; then
        reset_mode_default=2
    elif [ "$RESET_MODE" = "fixed_expire" ] && [ -n "$RESET_ANCHOR_DATE" ]; then
        reset_mode_default=3
    else
        reset_mode_default=1
    fi
    if [ "$RESET_MODE" = "anchored_monthly" ] && [ -n "$RESET_ANCHOR_DATE" ]; then
        printf -v current_reset_desc "每月 %s 日 %02d:%02d (首个重置: %s %02d:%02d)" \
            "$(echo "$RESET_ANCHOR_DATE" | cut -d- -f3)" "$RESET_HOUR" "$RESET_MINUTE" "$RESET_ANCHOR_DATE" "$RESET_HOUR" "$RESET_MINUTE"
    elif [ "$RESET_MODE" = "fixed_expire" ] && [ -n "$RESET_ANCHOR_DATE" ]; then
        printf -v current_reset_desc "固定到期日: %s %02d:%02d (到达后停机)" \
            "$RESET_ANCHOR_DATE" "$RESET_HOUR" "$RESET_MINUTE"
    else
        current_reset_desc="每月 1 日 00:00"
    fi
    read -rp "请输入选项 [当前: $current_reset_desc，默认: $reset_mode_default]: " reset_mode_choice
    reset_mode_choice=${reset_mode_choice:-$reset_mode_default}

    if [ "$reset_mode_choice" = "1" ]; then
        RESET_MODE="natural_month"
        RESET_ANCHOR_DATE=""
        RESET_HOUR=0
        RESET_MINUTE=0
        break
    elif [ "$reset_mode_choice" = "2" ] || [ "$reset_mode_choice" = "3" ]; then
        if [ "$reset_mode_choice" = "3" ]; then
            RESET_MODE="fixed_expire"
        else
            RESET_MODE="anchored_monthly"
        fi
        while true; do
            if [ "$RESET_MODE" = "fixed_expire" ]; then
                prompt_date="固定到期日期"
            else
                prompt_date="首个重置日期"
            fi
            if [ -n "$RESET_ANCHOR_DATE" ]; then
                read -rp "请输入$prompt_date (YYYY-MM-DD) [当前: $RESET_ANCHOR_DATE]: " input_anchor_date
                input_anchor_date=${input_anchor_date:-$RESET_ANCHOR_DATE}
            else
                read -rp "请输入$prompt_date (YYYY-MM-DD): " input_anchor_date
            fi

            if [ -n "${RESET_HOUR:-}" ] && [ -n "${RESET_MINUTE:-}" ]; then
                read -rp "请输入重置时间小时 (0-23) [当前: $RESET_HOUR]: " input_reset_hour
                read -rp "请输入重置时间分钟 (0-59) [当前: $RESET_MINUTE]: " input_reset_minute
                input_reset_hour=${input_reset_hour:-$RESET_HOUR}
                input_reset_minute=${input_reset_minute:-$RESET_MINUTE}
            else
                read -rp "请输入重置时间小时 (0-23，默认 0): " input_reset_hour
                read -rp "请输入重置时间分钟 (0-59，默认 0): " input_reset_minute
                input_reset_hour=${input_reset_hour:-0}
                input_reset_minute=${input_reset_minute:-0}
            fi

            if ! TZ="$TZ_NAME" date -d "$input_anchor_date" +%F >/dev/null 2>&1; then
                echo "错误: 日期格式无效，请输入 YYYY-MM-DD"
                continue
            fi

            if [[ ! "$input_reset_hour" =~ ^[0-9]+$ ]] || [ "$input_reset_hour" -lt 0 ] || [ "$input_reset_hour" -gt 23 ]; then
                echo "错误: 小时必须是 0-23 的整数"
                continue
            fi

            if [[ ! "$input_reset_minute" =~ ^[0-9]+$ ]] || [ "$input_reset_minute" -lt 0 ] || [ "$input_reset_minute" -gt 59 ]; then
                echo "错误: 分钟必须是 0-59 的整数"
                continue
            fi

            RESET_MODE="anchored_monthly"
            RESET_ANCHOR_DATE=$(TZ="$TZ_NAME" date -d "$input_anchor_date" +%F)
            RESET_HOUR="$input_reset_hour"
            RESET_MINUTE="$input_reset_minute"
            break
        done
        break
    else
        echo "错误: 无效选项，请输入 1、2 或 3"
    fi
done

if [ "$RESET_MODE" = "anchored_monthly" ]; then
    RESET_DAY=$(echo "$RESET_ANCHOR_DATE" | cut -d- -f3)
    RESET_DAY=$((10#$RESET_DAY))
else
    RESET_MODE="natural_month"
    RESET_DAY=1
    RESET_HOUR=0
    RESET_MINUTE=0
    RESET_ANCHOR_DATE=""
fi

printf -v RESET_TIME_HHMM "%02d:%02d" "$RESET_HOUR" "$RESET_MINUTE"
if [ "$RESET_MODE" = "anchored_monthly" ]; then
    RESET_DESC="每月 ${RESET_DAY} 日 ${RESET_TIME_HHMM} (首个重置: ${RESET_ANCHOR_DATE} ${RESET_TIME_HHMM}，短月自动按月末)"
elif [ "$RESET_MODE" = "fixed_expire" ]; then
    RESET_DESC="固定到期日: ${RESET_ANCHOR_DATE} ${RESET_TIME_HHMM} (到达后直接停机失效)"
else
    RESET_DESC="每月 1 日 00:00 (自然月默认)"
fi

# 自动获取默认网卡
DEFAULT_IFACE=$(ip route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="dev"){print $(i+1); exit}}')
if [ -z "$DEFAULT_IFACE" ]; then
    # 备选方案：获取第一个非 lo 网卡
    DEFAULT_IFACE=$(ip link show | grep -v "lo:" | awk -F: '/^[0-9]+:/{gsub(/ /, "", $2); print $2; exit}')
fi

# 网卡输入与验证
while true; do
    if [ -n "${IFACE:-}" ]; then
        read -rp "请确认出口网卡名称 [当前: $IFACE]: " input_iface
        if [ -z "$input_iface" ]; then
            break  # 保持原值
        fi
        IFACE="$input_iface"
    else
        if [ -n "$DEFAULT_IFACE" ]; then
            read -rp "请确认出口网卡名称 [默认: $DEFAULT_IFACE]: " IFACE
            IFACE=${IFACE:-$DEFAULT_IFACE}
        else
            read -rp "请输入出口网卡名称 (例如: eth0, ens4): " IFACE
        fi
    fi

    if [ -z "$IFACE" ]; then
        echo "错误: 网卡名称不能为空"
        IFACE=""
        continue
    fi

    # 验证网卡存在
    if [ ! -d "/sys/class/net/$IFACE" ]; then
        echo "错误: 网卡 $IFACE 不存在"
        echo "       可用网卡列表: $(ls /sys/class/net/ | tr '\n' ' ')"
        IFACE=""
        continue
    fi
    break
done

# 后端端口输入与验证 (默认 2080)
BACKEND_PORT="${BACKEND_PORT:-2080}"
while true; do
    read -rp "请输入后端服务端口 [默认: $BACKEND_PORT]: " input_port
    if [ -z "$input_port" ]; then
        break
    fi

    # 验证端口号
    if [[ ! "$input_port" =~ ^[0-9]+$ ]] || [ "$input_port" -lt 1 ] || [ "$input_port" -gt 65535 ]; then
        echo "错误: 端口号必须是 1-65535 之间的整数"
        continue
    fi

    BACKEND_PORT="$input_port"
    break
done

# 检查端口是否被占用 (排除自身 sub-server 服务)
if ! check_port_available "$BACKEND_PORT"; then
    # 检查是否是 sub-server 自身在占用
    PORT_OWNER=$(ss -tlnp 2>/dev/null | grep -E "[:.]${BACKEND_PORT}[[:space:]]" | grep -oP 'users:\(\("\K[^"]+' || true)
    if [ "$PORT_OWNER" = "sub_server.py" ] || [ "$PORT_OWNER" = "python3" ]; then
        echo "=> 端口 $BACKEND_PORT 被 sub-server 服务占用 (重新部署将自动重启)"
    else
        echo "警告: 端口 $BACKEND_PORT 已被其他进程占用 (${PORT_OWNER:-未知})"
        read -rp "是否继续使用此端口? [y/N]: " continue_port
        if [[ ! "$continue_port" =~ ^[Yy] ]]; then
            echo "已取消安装"
            exit 1
        fi
    fi
fi

if [ "$RESET_MODE" != "anchored_monthly" ] && [ "$RESET_MODE" != "fixed_expire" ]; then
    RESET_ANCHOR_DATE=""
fi

# 生成随机 Token (如果未从配置文件加载)
if [ -z "${TOKEN:-}" ]; then
    TOKEN="$(openssl rand -hex 24 2>/dev/null || tr -dc 'a-f0-9' < /dev/urandom | head -c 48)"
fi
echo
echo "=> 安全访问 Token: $TOKEN"
echo "=================================================="

# ===================== 配置摘要与确认 =====================
echo ""
echo "========== 配置摘要 =========="
echo "域名:       $DOMAIN"
echo "用户名:     $CADDY_USER"
echo "密码:       $([ "$CADDY_PASS" = "<已保存的密码>" ] && echo "<已保存>" || echo "${CADDY_PASS:0:3}***")"
echo "流量上限:   $TRAFFIC_LIMIT_GIB GiB"
echo "已使用流量: ${USED_TRAFFIC_GIB:-0} GiB"
echo "时区:       $TZ_NAME"
echo "刷新规则:   $RESET_DESC"
echo "网卡:       $IFACE"
echo "后端端口:   $BACKEND_PORT"
echo "Token:      ${TOKEN:0:12}...${TOKEN: -12}"
echo "=============================="
echo ""
read -rp "确认开始安装? [Y/n]: " confirm
confirm=${confirm:-Y}
if [[ ! "$confirm" =~ ^[Yy] ]]; then
    echo "已取消安装"
    exit 0
fi
echo ""

# ===================== 开始安装 =====================

echo "[1/8] 安装基础依赖 (vnstat, python3, curl, openssl)..."
apt-get update -qq
apt-get install -y -qq vnstat python3 python3-pip curl jq openssl debian-keyring debian-archive-keyring apt-transport-https gpg
# 安装终端二维码库 (用于部署完成后展示订阅链接二维码)
python3 -m pip install -q --break-system-packages "qrcode[tty]" 2>/dev/null \
    || python3 -m pip install -q "qrcode[tty]" 2>/dev/null \
    || true

echo "[2/8] 安装 Caddy (通过官方 APT 仓库)..."
# 使用 Caddy 官方 APT 仓库，确保稳定可靠
if ! command -v caddy &>/dev/null; then
    # 幂等处理：先删除旧 key 再导入，避免重复运行报错
    rm -f /usr/share/keyrings/caddy-stable-archive-keyring.gpg
    curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
        | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
    curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
        | tee /etc/apt/sources.list.d/caddy-stable.list > /dev/null
    apt-get update -qq
    apt-get install -y -qq caddy
else
    echo "=> Caddy 已安装，跳过 (当前版本: $(caddy version))"
fi

# 启动基础服务
echo "=> 启动 vnstat 和 Caddy 服务..."
systemctl enable --now vnstat
systemctl enable --now caddy

# 保存配置到文件
echo "=> 保存配置到 $CONFIG_FILE ..."
save_config

echo "[3/8] 配置 vnStat 监控网卡与时区..."
# 只在 Interface 行存在时替换，否则追加
if grep -q '^Interface ' /etc/vnstat.conf 2>/dev/null; then
    sed -i "s/^Interface .*/Interface \"$IFACE\"/" /etc/vnstat.conf
else
    echo "Interface \"$IFACE\"" >> /etc/vnstat.conf
fi
mkdir -p /etc/systemd/system/vnstat.service.d
cat > /etc/systemd/system/vnstat.service.d/override.conf <<EOF
[Service]
Environment=TZ=$TZ_NAME
EOF
systemctl daemon-reload
systemctl restart vnstat

echo "[4/8] 创建隔离的服务用户和目录..."
id subsrv &>/dev/null || useradd -r -s /usr/sbin/nologin subsrv
mkdir -p /var/lib/subsrv
chown subsrv:subsrv /var/lib/subsrv
chmod 750 /var/lib/subsrv

# 初始化订阅配置副本 — Clash Meta (YAML)
if [ -f /etc/s-box/clash_meta_client.yaml ]; then
    cp -f /etc/s-box/clash_meta_client.yaml /var/lib/subsrv/client.yaml
else
    echo "# 暂无订阅内容，等待 yonggekkk 脚本生成" > /var/lib/subsrv/client.yaml
    echo "=> 警告: /etc/s-box/clash_meta_client.yaml 不存在，已创建默认空配置"
fi
chown subsrv:subsrv /var/lib/subsrv/client.yaml
chmod 640 /var/lib/subsrv/client.yaml

# 初始化订阅配置副本 — sing-box (JSON)
if [ -f /etc/s-box/sing_box_client.json ]; then
    cp -f /etc/s-box/sing_box_client.json /var/lib/subsrv/client.json
else
    echo '{"log":{"level":"warn"},"dns":{},"inbounds":[],"outbounds":[]}' > /var/lib/subsrv/client.json
    echo "=> 警告: /etc/s-box/sing_box_client.json 不存在，已创建默认空配置"
fi
chown subsrv:subsrv /var/lib/subsrv/client.json
chmod 640 /var/lib/subsrv/client.json

# 初始化订阅配置副本 — Shadowrocket (TXT)
if [ -f /etc/s-box/jhdy.txt ]; then
    cp -f /etc/s-box/jhdy.txt /var/lib/subsrv/client.txt
elif [ -f /etc/s-box/jh_sub.txt ]; then
    cp -f /etc/s-box/jh_sub.txt /var/lib/subsrv/client.txt
    echo "=> 提示: jhdy.txt 不存在，已使用 jh_sub.txt 代替"
else
    echo "# 暂无订阅内容，等待 yonggekkk 脚本生成" > /var/lib/subsrv/client.txt
    echo "=> 警告: /etc/s-box/jhdy.txt 和 jh_sub.txt 均不存在，已创建默认空配置"
fi
chown subsrv:subsrv /var/lib/subsrv/client.txt
chmod 640 /var/lib/subsrv/client.txt

# 初始化流量状态文件（仅在不存在时创建空文件，保留已有内容）
if [ ! -f /var/lib/subsrv/tx_state.json ]; then
    touch /var/lib/subsrv/tx_state.json
fi
chown subsrv:subsrv /var/lib/subsrv/tx_state.json
chmod 640 /var/lib/subsrv/tx_state.json

echo "[5/8] 配置配置文件定时同步 (每 5 分钟)..."
cat > /usr/local/bin/refresh_sub_copy.sh <<'SH'
#!/usr/bin/env bash
set -euo pipefail

# 同步 Clash Meta 配置 (YAML)
SRC_YAML="/etc/s-box/clash_meta_client.yaml"
DST_YAML="/var/lib/subsrv/client.yaml"
if [ -f "$SRC_YAML" ]; then
    TMP_YAML="/var/lib/subsrv/client.yaml.tmp"
    cp -f "$SRC_YAML" "$TMP_YAML"
    chown subsrv:subsrv "$TMP_YAML"
    chmod 640 "$TMP_YAML"
    mv -f "$TMP_YAML" "$DST_YAML"
fi

# 同步 sing-box 配置 (JSON)
SRC_JSON="/etc/s-box/sing_box_client.json"
DST_JSON="/var/lib/subsrv/client.json"
if [ -f "$SRC_JSON" ]; then
    TMP_JSON="/var/lib/subsrv/client.json.tmp"
    cp -f "$SRC_JSON" "$TMP_JSON"
    chown subsrv:subsrv "$TMP_JSON"
    chmod 640 "$TMP_JSON"
    mv -f "$TMP_JSON" "$DST_JSON"
fi

# 同步 Shadowrocket 配置 (TXT)
SRC_TXT="/etc/s-box/jhdy.txt"
SRC_TXT_FALLBACK="/etc/s-box/jh_sub.txt"
DST_TXT="/var/lib/subsrv/client.txt"
if [ -f "$SRC_TXT" ]; then
    TMP_TXT="/var/lib/subsrv/client.txt.tmp"
    cp -f "$SRC_TXT" "$TMP_TXT"
    chown subsrv:subsrv "$TMP_TXT"
    chmod 640 "$TMP_TXT"
    mv -f "$TMP_TXT" "$DST_TXT"
elif [ -f "$SRC_TXT_FALLBACK" ]; then
    TMP_TXT="/var/lib/subsrv/client.txt.tmp"
    cp -f "$SRC_TXT_FALLBACK" "$TMP_TXT"
    chown subsrv:subsrv "$TMP_TXT"
    chmod 640 "$TMP_TXT"
    mv -f "$TMP_TXT" "$DST_TXT"
fi
SH
chmod +x /usr/local/bin/refresh_sub_copy.sh

cat > /etc/systemd/system/refresh-sub-copy.service <<'UNIT'
[Unit]
Description=Refresh served subscription copy
[Service]
Type=oneshot
ExecStart=/usr/local/bin/refresh_sub_copy.sh
UNIT

cat > /etc/systemd/system/refresh-sub-copy.timer <<'UNIT'
[Unit]
Description=Run refresh-sub-copy every 5 minutes
[Timer]
OnBootSec=1min
OnUnitActiveSec=5min
Persistent=true
[Install]
WantedBy=timers.target
UNIT

echo "[6/8] 配置流量基线重置机制..."
# 使用 printf 代替嵌套 heredoc，避免 heredoc 标记冲突
cat > /usr/local/bin/reset_tx_baseline.sh <<SH
#!/usr/bin/env bash
set -euo pipefail
IFACE="\${1:-$IFACE}"
STATE="/var/lib/subsrv/tx_state.json"
TZNAME="$TZ_NAME"
RESET_MODE="$RESET_MODE"
RESET_ANCHOR_DATE="$RESET_ANCHOR_DATE"
RESET_DAY="$RESET_DAY"
RESET_HOUR="$RESET_HOUR"
RESET_MINUTE="$RESET_MINUTE"

calc_cycle_key() {
    local now_ts y m d hh mm
    now_ts="\$(TZ=\$TZNAME date +%s)"
    y="\$(TZ=\$TZNAME date +%Y)"
    m="\$(TZ=\$TZNAME date +%m)"

    if [ "\$RESET_MODE" = "natural_month" ]; then
        printf "%04d-%02d-%02dT%02d:%02d" "\$((10#\$y))" "\$((10#\$m))" 1 0 0
        return
    fi

    if [ "\$RESET_MODE" = "fixed_expire" ]; then
        printf "fixed:%sT%02d:%02d" "\$RESET_ANCHOR_DATE" "\$((10#\$RESET_HOUR))" "\$((10#\$RESET_MINUTE))"
        return
    fi

    d="\$RESET_DAY"
    hh="\$RESET_HOUR"
    mm="\$RESET_MINUTE"

    if [ -n "\$RESET_ANCHOR_DATE" ]; then
        local anchor_ts
        anchor_ts="\$(TZ=\$TZNAME date -d "\$RESET_ANCHOR_DATE \$hh:\$mm:00" +%s 2>/dev/null || echo 0)"
        if [ "\$now_ts" -lt "\$anchor_ts" ]; then
            printf "pre-anchor:%sT%02d:%02d" "\$RESET_ANCHOR_DATE" "\$((10#\$hh))" "\$((10#\$mm))"
            return
        fi
    fi

    local this_last this_d this_cycle this_ts
    this_last="\$(TZ=\$TZNAME date -d "\$((10#\$y))-\$((10#\$m))-01 +1 month -1 day" +%d)"
    this_d="\$d"
    if [ "\$this_d" -gt "\$this_last" ]; then
        this_d="\$this_last"
    fi
    printf -v this_cycle "%04d-%02d-%02d %02d:%02d:00" "\$((10#\$y))" "\$((10#\$m))" "\$((10#\$this_d))" "\$((10#\$hh))" "\$((10#\$mm))"
    this_ts="\$(TZ=\$TZNAME date -d "\$this_cycle" +%s)"

    if [ "\$now_ts" -ge "\$this_ts" ]; then
        printf "%04d-%02d-%02dT%02d:%02d" "\$((10#\$y))" "\$((10#\$m))" "\$((10#\$this_d))" "\$((10#\$hh))" "\$((10#\$mm))"
        return
    fi

    local prev_y prev_m prev_last prev_d
    prev_y="\$((10#\$y))"
    prev_m="\$((10#\$m - 1))"
    if [ "\$prev_m" -eq 0 ]; then
        prev_m=12
        prev_y="\$((prev_y - 1))"
    fi
    prev_last="\$(TZ=\$TZNAME date -d "\$prev_y-\$prev_m-01 +1 month -1 day" +%d)"
    prev_d="\$d"
    if [ "\$prev_d" -gt "\$prev_last" ]; then
        prev_d="\$prev_last"
    fi
    printf "%04d-%02d-%02dT%02d:%02d" "\$prev_y" "\$prev_m" "\$((10#\$prev_d))" "\$((10#\$hh))" "\$((10#\$mm))"
}

cycle_key="\$(calc_cycle_key)"
tx="\$(cat /sys/class/net/"\$IFACE"/statistics/tx_bytes 2>/dev/null || echo 0)"

if [[ "\$cycle_key" == fixed:* ]]; then
    now_ts="\$(TZ=\$TZNAME date +%s)"
    anchor_ts="\$(TZ=\$TZNAME date -d "\$RESET_ANCHOR_DATE \$RESET_HOUR:\$RESET_MINUTE:00" +%s 2>/dev/null || echo 0)"
    if [ "\$now_ts" -ge "\$anchor_ts" ]; then
        echo "[reset_tx_baseline] \$(date -Is) EXPIRED! Stopping proxy services..."
        systemctl stop sing-box clash-meta xray v2ray caddy sub-server 2>/dev/null || true
        exit 0
    fi
fi

if [[ "\$cycle_key" == pre-anchor:* ]]; then
    echo "[reset_tx_baseline] \$(date -Is) IFACE=\$IFACE next_anchor=\${cycle_key#pre-anchor:} not reached, skip"
    exit 0
fi

saved_cycle_key=""
if [ -f "\$STATE" ] && [ -s "\$STATE" ]; then
    saved_cycle_key="\$(python3 - "\$STATE" <<'PYEOF'
import json, sys
try:
    with open(sys.argv[1], encoding='utf-8') as f:
        st = json.load(f)
    ck = st.get('cycle_key')
    if ck:
        print(ck)
    else:
        ym = st.get('ym', '')
        if ym:
            print(f"{ym}-01T00:00")
except Exception:
    pass
PYEOF
    )"
fi

if [ "\$saved_cycle_key" = "\$cycle_key" ]; then
    # cycle_key 匹配，检查计数器是否回绕（重启后 tx_bytes 归零）
    saved_base="\$(python3 - "\$STATE" <<'PYEOF'
import json, sys
try:
    with open(sys.argv[1], encoding='utf-8') as f:
        st = json.load(f)
    b = st.get('base_tx')
    if b is not None:
        print(int(b))
except Exception:
    pass
PYEOF
    )"
    if [ -n "\$saved_base" ] && [ "\$tx" -lt "\$saved_base" ]; then
        # 计数器回绕：cur_tx < base_tx，重置基线
        tmp="\$(mktemp)"
        printf '{"cycle_key":"%s","base_tx":%s}\n' "\$cycle_key" "\$tx" > "\$tmp"
        install -o subsrv -g subsrv -m 640 "\$tmp" "\$STATE"
        rm -f "\$tmp"
        echo "[reset_tx_baseline] \$(date -Is) IFACE=\$IFACE cycle_key=\$cycle_key base_tx=\$tx wrote=\$STATE (reason: counter wrapped, old_base=\$saved_base)"
        exit 0
    fi
    echo "[reset_tx_baseline] \$(date -Is) IFACE=\$IFACE cycle_key=\$cycle_key already up-to-date, skip"
    exit 0
fi

tmp="\$(mktemp)"
printf '{"cycle_key":"%s","base_tx":%s}\n' "\$cycle_key" "\$tx" > "\$tmp"
install -o subsrv -g subsrv -m 640 "\$tmp" "\$STATE"
rm -f "\$tmp"
echo "[reset_tx_baseline] \$(date -Is) IFACE=\$IFACE cycle_key=\$cycle_key base_tx=\$tx wrote=\$STATE"
SH
chmod +x /usr/local/bin/reset_tx_baseline.sh

# 流量基线初始化：根据用户设置的"已使用流量"写入 tx_state.json
STATE_FILE="/var/lib/subsrv/tx_state.json"
NEED_RESET=true

if [ -f "$STATE_FILE" ] && [ -s "$STATE_FILE" ]; then
    HAS_BASELINE=$(python3 -c "
import json, sys
try:
    with open(sys.argv[1]) as f:
        st = json.load(f)
    print('yes' if ('base_tx' in st and ('cycle_key' in st or 'ym' in st)) else 'no')
except:
    print('no')
" "$STATE_FILE")
    if [ "$HAS_BASELINE" = "yes" ]; then
        NEED_RESET=false
        echo "=> 检测到已有流量基线，跳过初始化以保留已累计流量"
    else
        echo "=> 检测到状态文件损坏或缺少基线信息，将重新初始化"
    fi
fi

# 如果用户指定了已使用流量 (USED_TRAFFIC_GIB > 0)，
# 则无论是否首次部署，均根据该值重新计算基线并写入状态文件
_USED_GIB="${USED_TRAFFIC_GIB:-0}"
if python3 -c "import sys; sys.exit(0 if float(sys.argv[1]) > 0 else 1)" "$_USED_GIB" 2>/dev/null; then
    echo "=> 根据已使用流量 ${_USED_GIB} GiB 重新计算流量基线..."
    python3 - "$STATE_FILE" "$_USED_GIB" "$IFACE" "$RESET_MODE" "${RESET_ANCHOR_DATE:-}" "$RESET_DAY" "$RESET_HOUR" "$RESET_MINUTE" "$TZ_NAME" <<'PYEOF'
import sys, os, json
from datetime import datetime, timezone
from calendar import monthrange

state_path        = sys.argv[1]
used_gib          = float(sys.argv[2])
iface             = sys.argv[3]
reset_mode        = sys.argv[4]
reset_anchor_date = sys.argv[5]
reset_day         = int(sys.argv[6])
reset_hour        = int(sys.argv[7])
reset_minute      = int(sys.argv[8])
tz_name           = sys.argv[9]
used_bytes        = int(used_gib * 1024 ** 3)

try:
    from zoneinfo import ZoneInfo
    tz = ZoneInfo(tz_name)
except Exception:
    tz = timezone.utc

def shift_month(year, month, delta):
    total = year * 12 + (month - 1) + delta
    y = total // 12
    m = total % 12 + 1
    return y, m

def cycle_start_for(year, month):
    if reset_mode == "natural_month":
        d, hh, mm = 1, 0, 0
    else:
        last = monthrange(year, month)[1]
        d  = min(max(reset_day, 1), last)
        hh = min(max(reset_hour, 0), 23)
        mm = min(max(reset_minute, 0), 59)
    return datetime(year, month, d, hh, mm, 0, tzinfo=tz)

now = datetime.now(tz)
if reset_mode == "fixed_expire":
    cycle_key = f"fixed:{reset_anchor_date}T{reset_hour:02d}:{reset_minute:02d}"
else:
    this_cycle = cycle_start_for(now.year, now.month)
    if now >= this_cycle:
        cycle_start = this_cycle
    else:
        py, pm = shift_month(now.year, now.month, -1)
        cycle_start = cycle_start_for(py, pm)
    cycle_key = cycle_start.strftime("%Y-%m-%dT%H:%M")

# 读取当前网卡 tx_bytes
cur_tx = 0
try:
    with open(f"/sys/class/net/{iface}/statistics/tx_bytes", encoding="utf-8") as f:
        cur_tx = int(f.read().strip())
except Exception as e:
    print(f"[baseline] WARN: cannot read tx_bytes for {iface}: {e}", flush=True)

# base_tx = cur_tx - used_bytes
# 允许负值：当用户设置的已用流量超过当前计数器（例如 VPS 重启后计数器归零，
# 但本周期实际已用流量很大），base_tx 为负数可正确表达偏移量。
# used = cur_tx - base_tx = cur_tx - (cur_tx - used_bytes) = used_bytes ✓
new_base = cur_tx - used_bytes

tmp = state_path + ".tmp"
with open(tmp, "w", encoding="utf-8") as f:
    json.dump({"cycle_key": cycle_key, "base_tx": new_base}, f, separators=(",", ":"))
import subprocess
ret = subprocess.run(
    ["install", "-o", "subsrv", "-g", "subsrv", "-m", "640", tmp, state_path],
    capture_output=True
)
if ret.returncode != 0:
    # install 失败时退回到直接移动（root 写入），并修正属主和权限
    import shutil
    shutil.move(tmp, state_path)
    try:
        import pwd, grp
        uid = pwd.getpwnam("subsrv").pw_uid
        gid = grp.getgrnam("subsrv").gr_gid
        os.chown(state_path, uid, gid)
    except Exception:
        pass
    os.chmod(state_path, 0o640)
    print(f"[baseline] WARN: install failed ({ret.stderr.decode().strip()}), wrote as root fallback", flush=True)
else:
    try:
        os.unlink(tmp)
    except Exception:
        pass
print(f"[baseline] set: iface={iface} cur_tx={cur_tx} used_bytes={used_bytes} new_base={new_base} cycle_key={cycle_key}", flush=True)
PYEOF
elif [ "$NEED_RESET" = true ]; then
    /usr/local/bin/reset_tx_baseline.sh "$IFACE"
fi

# 设置系统时区以确保 systemd timer 在正确时间触发
echo "=> 设置系统时区为 $TZ_NAME (确保 Timer 在正确时间触发)..."
timedatectl set-timezone "$TZ_NAME" 2>/dev/null || {
    ln -sf "/usr/share/zoneinfo/$TZ_NAME" /etc/localtime
    echo "$TZ_NAME" > /etc/timezone
}

cat > /etc/systemd/system/reset-tx-baseline.service <<UNIT
[Unit]
Description=Reset monthly tx baseline
[Service]
Type=oneshot
Environment=TZ=$TZ_NAME
ExecStart=/usr/local/bin/reset_tx_baseline.sh $IFACE
UNIT

cat > /etc/systemd/system/reset-tx-baseline.timer <<UNIT
[Unit]
Description=Run reset-tx-baseline every minute (guarded by cycle key)
[Timer]
OnCalendar=*-*-* *:*:00
Persistent=true
[Install]
WantedBy=timers.target
UNIT

echo "[7/8] 编写并启动动态订阅服务端 (Python)..."
cat > /usr/local/bin/sub_server.py <<'PY'
#!/usr/bin/env python3
import json, os
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import urlparse
from datetime import datetime, timezone
from calendar import monthrange

try:
    from zoneinfo import ZoneInfo
except Exception:
    ZoneInfo = None

def log(msg):
    ts = datetime.now(timezone.utc).isoformat()
    print(f"[sub_server] {ts} {msg}", flush=True)

IFACE      = os.environ.get("SUB_IFACE",      "ens4")
YAML_TOKEN_PATH = os.environ.get("SUB_TOKEN_PATH",  "/sub/token.yaml")
JSON_TOKEN_PATH = os.environ.get("SUB_JSON_TOKEN_PATH", "/sub/token.json")
YAML_PATH  = os.environ.get("SUB_YAML_PATH",   "/var/lib/subsrv/client.yaml")
JSON_PATH  = os.environ.get("SUB_JSON_PATH",   "/var/lib/subsrv/client.json")
TXT_TOKEN_PATH = os.environ.get("SUB_TXT_TOKEN_PATH", "/sub/token.txt")
TXT_PATH   = os.environ.get("SUB_TXT_PATH",   "/var/lib/subsrv/client.txt")
LIMIT_GIB  = float(os.environ.get("SUB_LIMIT_GIB", "0"))
TZ_NAME    = os.environ.get("SUB_TZ",          "America/Los_Angeles")
STATE_PATH = os.environ.get("SUB_STATE_PATH",  "/var/lib/subsrv/tx_state.json")
RESET_MODE = os.environ.get("SUB_RESET_MODE", "natural_month")
RESET_ANCHOR_DATE = os.environ.get("SUB_RESET_ANCHOR_DATE", "")
RESET_DAY = int(os.environ.get("SUB_RESET_DAY", "1"))
RESET_HOUR = int(os.environ.get("SUB_RESET_HOUR", "0"))
RESET_MINUTE = int(os.environ.get("SUB_RESET_MINUTE", "0"))

# 0 表示无限流量，用 999 TiB 作为显示值 (客户端会显示几乎用不完的额度)
if LIMIT_GIB <= 0:
    TOTAL_BYTES = int(999 * 1024 * 1024 * 1024 * 1024)  # 999 TiB
else:
    TOTAL_BYTES = int(LIMIT_GIB * 1024 * 1024 * 1024)

# 路径 -> (文件路径, Content-Type) 的映射表
ROUTE_MAP = {
    YAML_TOKEN_PATH: (YAML_PATH, "text/yaml; charset=utf-8"),
    JSON_TOKEN_PATH: (JSON_PATH, "application/json; charset=utf-8"),
    TXT_TOKEN_PATH: (TXT_PATH, "text/plain; charset=utf-8"),
}

def pt_now():
    if ZoneInfo:
        return datetime.now(ZoneInfo(TZ_NAME))
    return datetime.now(timezone.utc)

def shift_month(year, month, delta):
    total = year * 12 + (month - 1) + delta
    y = total // 12
    m = total % 12 + 1
    return y, m

def cycle_start_for(year, month):
    if RESET_MODE == "natural_month":
        d = 1
        hh = 0
        mm = 0
    else:
        last = monthrange(year, month)[1]
        d = min(max(RESET_DAY, 1), last)
        hh = min(max(RESET_HOUR, 0), 23)
        mm = min(max(RESET_MINUTE, 0), 59)

    if ZoneInfo:
        return datetime(year, month, d, hh, mm, 0, tzinfo=ZoneInfo(TZ_NAME))
    return datetime(year, month, d, hh, mm, 0, tzinfo=timezone.utc)

def anchor_dt():
    if RESET_MODE != "anchored_monthly" or not RESET_ANCHOR_DATE:
        return None
    try:
        y, m, d = map(int, RESET_ANCHOR_DATE.split("-"))
    except Exception:
        return None

    hh = min(max(RESET_HOUR, 0), 23)
    mm = min(max(RESET_MINUTE, 0), 59)
    if ZoneInfo:
        return datetime(y, m, d, hh, mm, 0, tzinfo=ZoneInfo(TZ_NAME))
    return datetime(y, m, d, hh, mm, 0, tzinfo=timezone.utc)

def current_cycle_start(now=None):
    now = now or pt_now()
    if RESET_MODE == "fixed_expire":
        adt = anchor_dt()
        if adt: return adt
        return datetime(2000, 1, 1, tzinfo=timezone.utc)
    this_cycle = cycle_start_for(now.year, now.month)
    if now >= this_cycle:
        return this_cycle
    py, pm = shift_month(now.year, now.month, -1)
    return cycle_start_for(py, pm)

def cycle_key_from_dt(dt):
    if RESET_MODE == "fixed_expire":
        return f"fixed:{dt.strftime('%Y-%m-%dT%H:%M')}"
    return dt.strftime("%Y-%m-%dT%H:%M")

def next_reset_epoch_pt():
    now = pt_now()
    adt = anchor_dt()
    if RESET_MODE == "fixed_expire":
        if adt:
            return int(adt.timestamp())
        return 0
    if adt and now < adt:
        return int(adt.timestamp())

    this_cycle = cycle_start_for(now.year, now.month)
    if now < this_cycle:
        next_cycle = this_cycle
    else:
        ny, nm = shift_month(now.year, now.month, 1)
        next_cycle = cycle_start_for(ny, nm)
    return int(next_cycle.timestamp())

def read_tx_bytes_sysfs():
    p = f"/sys/class/net/{IFACE}/statistics/tx_bytes"
    try:
        with open(p, "r", encoding="utf-8") as f:
            return int(f.read().strip())
    except Exception as e:
        log(f"WARN: cannot read {p}: {e}")
        return 0

def load_state():
    try:
        with open(STATE_PATH, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}

def get_state_cycle_key(st):
    ck = st.get("cycle_key")
    if isinstance(ck, str) and ck:
        return ck
    ym = st.get("ym")
    if isinstance(ym, str) and len(ym) == 7:
        return f"{ym}-01T00:00"
    return None

def month_used_tx_bytes_realtime():
    """只读计算本周期已用流量，不写入状态文件。
    状态文件的创建/更新由 reset_tx_baseline.sh (定时任务) 和部署脚本独占。
    """
    cur = read_tx_bytes_sysfs()
    st = load_state()
    st_cycle_key = get_state_cycle_key(st)
    base = st.get("base_tx")

    # 状态文件缺失或损坏：等待 reset_tx_baseline.sh 初始化
    if st_cycle_key is None or base is None:
        log(f"state not ready: st_cycle_key={st_cycle_key} base={base} (waiting for reset_tx_baseline.sh)")
        return 0, cur, cur

    used = cur - int(base)

    # 计数器回绕（重启后 cur_tx < base_tx）：返回 0，等待定时任务修正
    if used < 0:
        log(f"counter wrapped: cur={cur} base={base} used={used} (waiting for reset_tx_baseline.sh)")
        return 0, int(base), cur

    return int(used), int(base), int(cur)

class Handler(BaseHTTPRequestHandler):
    def do_HEAD(self):
        self._head_only = True
        return self.do_GET()

    def do_GET(self):
        self._head_only = getattr(self, "_head_only", False)
        path = urlparse(self.path).path
        log(f"{'HEAD' if self._head_only else 'GET'} {path} from {self.client_address[0]}")

        # 查找路由
        route = ROUTE_MAP.get(path)
        if route is None:
            self.send_response(404)
            self.end_headers()
            return

        file_path, content_type = route

        try:
            used_tx, base_tx, cur_tx = month_used_tx_bytes_realtime()
        except Exception as e:
            log(f"ERROR reading tx_bytes: {e}")
            used_tx, base_tx, cur_tx = 0, 0, 0

        expire = next_reset_epoch_pt()
        remain = max(TOTAL_BYTES - used_tx, 0)

        is_expired = False
        if RESET_MODE == "fixed_expire" and expire > 0:
            if pt_now().timestamp() >= expire:
                is_expired = True

        try:
            if is_expired:
                if content_type.startswith("application/json"):
                    body = b'{"log":{"level":"warn"},"dns":{},"inbounds":[],"outbounds":[]}\n'
                else:
                    body = b"# Subscription expired. Your plan has ended.\n"
                log("Subscription expired, returning empty config")
            else:
                with open(file_path, "rb") as f:
                    body = f.read()
                log(f"read ok: {file_path} bytes={len(body)}")
        except Exception as e:
            log(f"ERROR read file: {e}")
            if content_type.startswith("application/json"):
                body = b'{"error":"subscription source missing"}\n'
            else:
                body = b"# subscription source missing\n"

        header_val = f"upload=0; download={used_tx}; total={TOTAL_BYTES}; expire={expire}"
        log(f"userinfo: used_tx={used_tx} remain={remain} cycle={cycle_key_from_dt(current_cycle_start())} base_tx={base_tx} cur_tx={cur_tx}")

        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Cache-Control", "no-cache, no-store, must-revalidate")
        self.send_header("Pragma", "no-cache")
        self.send_header("Expires", "0")
        self.send_header("subscription-userinfo", header_val)
        self.end_headers()

        if not self._head_only:
            self.wfile.write(body)

    def log_message(self, fmt, *args):
        return

def main():
    host = os.environ.get("SUB_LISTEN", "127.0.0.1")
    port = int(os.environ.get("SUB_PORT", "2080"))
    log(f"start listen={host}:{port} iface={IFACE} tz={TZ_NAME}")
    log(f"  yaml: {YAML_TOKEN_PATH} -> {YAML_PATH}")
    log(f"  json: {JSON_TOKEN_PATH} -> {JSON_PATH}")
    log(f"  txt:  {TXT_TOKEN_PATH} -> {TXT_PATH}")
    log(f"  state={STATE_PATH}")
    HTTPServer((host, port), Handler).serve_forever()

if __name__ == "__main__":
    main()
PY
chmod +x /usr/local/bin/sub_server.py

cat > /etc/systemd/system/sub-server.service <<UNIT
[Unit]
Description=Dynamic subscription server with subscription-userinfo
After=network-online.target vnstat.service
Wants=network-online.target

[Service]
User=subsrv
Group=subsrv
Environment=SUB_IFACE=$IFACE
Environment=SUB_TOKEN_PATH=/sub/$TOKEN.yaml
Environment=SUB_JSON_TOKEN_PATH=/sub/$TOKEN.json
Environment=SUB_TXT_TOKEN_PATH=/sub/$TOKEN.txt
Environment=SUB_YAML_PATH=/var/lib/subsrv/client.yaml
Environment=SUB_JSON_PATH=/var/lib/subsrv/client.json
Environment=SUB_TXT_PATH=/var/lib/subsrv/client.txt
Environment=SUB_LIMIT_GIB=$TRAFFIC_LIMIT_GIB
Environment=SUB_TZ=$TZ_NAME
Environment=SUB_STATE_PATH=/var/lib/subsrv/tx_state.json
Environment=SUB_RESET_MODE=$RESET_MODE
Environment=SUB_RESET_ANCHOR_DATE=$RESET_ANCHOR_DATE
Environment=SUB_RESET_DAY=$RESET_DAY
Environment=SUB_RESET_HOUR=$RESET_HOUR
Environment=SUB_RESET_MINUTE=$RESET_MINUTE
Environment=SUB_LISTEN=127.0.0.1
Environment=SUB_PORT=$BACKEND_PORT
ExecStart=/usr/local/bin/sub_server.py
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable --now refresh-sub-copy.timer
# sub_server.py 已改为只读模式，不再写入 tx_state.json
# 即使 Persistent=true 导致 reset_tx_baseline.sh 立即触发也无害（cycle_key 匹配会跳过）
systemctl enable --now reset-tx-baseline.timer
# 使用 restart 而非 start，确保覆盖部署时环境变量生效
systemctl enable sub-server
systemctl restart sub-server

echo "[8/8] 配置 Caddy (反向代理与鉴权)..."
# 生成密码哈希 (如果是新密码)
if [ "$NEED_NEW_PASSWORD" = true ]; then
    PASSWORD_HASH=$(caddy hash-password --plaintext "$CADDY_PASS" 2>/dev/null | tr -d '[:space:]')
    if [ -z "$PASSWORD_HASH" ]; then
        echo "错误: caddy hash-password 生成失败，请检查 Caddy 是否正确安装"
        exit 1
    fi
    # 更新配置文件中的密码哈希
    echo "=> 更新配置文件中的密码哈希..."
    save_config
else
    echo "=> 使用已保存的密码哈希: ${PASSWORD_HASH:0:20}..."
fi

# 备份旧配置
cp -a /etc/caddy/Caddyfile "/etc/caddy/Caddyfile.bak.$(date +%F_%H%M%S)" 2>/dev/null || true

# ==========================================
# Caddyfile 使用 handle 块实现互斥路由
# ==========================================
# Caddy 的指令优先级规则: 同级别的 respond / reverse_proxy / basic_auth
# 不保证按书写顺序执行。使用 handle 块可以创建互斥的路由分组:
#   - handle @matcher1 { ... }  优先匹配
#   - handle @matcher2 { ... }  次优先
#   - handle { ... }            兜底 (其他所有请求)
#
# 路由策略:
#   1. ?token=TOKEN 参数访问 -> 免 BasicAuth (给 CMFA 等客户端)
#   2. 精确路径访问 -> 需要 BasicAuth (给 Clash Party / 浏览器)
#   3. 其他所有请求 -> 404
# ==========================================
# 注意：PASSWORD_HASH 是 bcrypt 格式，含有 $ 字符，不能直接放入 <<EOF heredoc
# (shell 会把 $2a、$14 等当作变量展开导致哈希损坏)
# 解决方案：先用占位符写模板，再用 printf %s 原样替换密码哈希
cat > /etc/caddy/Caddyfile <<EOF
$DOMAIN {
	# 订阅文件的精确路径 (Clash Meta YAML + sing-box JSON + Shadowrocket TXT)
	@sub_path {
		path /sub/$TOKEN.yaml /sub/$TOKEN.json /sub/$TOKEN.txt
	}

	# token 参数免密访问 (给 CMFA / SFA / Shadowrocket 等不支持 BasicAuth 的客户端)
	@sub_with_token {
		path /sub/$TOKEN.yaml /sub/$TOKEN.json /sub/$TOKEN.txt
		query token=$TOKEN
	}

	# 1) token 参数优先：不需要 BasicAuth
	handle @sub_with_token {
		reverse_proxy 127.0.0.1:$BACKEND_PORT
	}

	# 2) 精确路径匹配：需要 BasicAuth
	handle @sub_path {
		basic_auth {
			$CADDY_USER __PASSWORD_HASH_PLACEHOLDER__
		}
		reverse_proxy 127.0.0.1:$BACKEND_PORT
	}

	# 3) 其他路径全部 404
	handle {
		respond "not found" 404
	}
}
EOF

# 用 python3 原样替换占位符 (避免 sed 特殊字符转义问题)
python3 - "$PASSWORD_HASH" <<'PYEOF'
import sys, pathlib
h = sys.argv[1]
p = pathlib.Path("/etc/caddy/Caddyfile")
p.write_text(p.read_text().replace("__PASSWORD_HASH_PLACEHOLDER__", h))
PYEOF

caddy fmt --overwrite /etc/caddy/Caddyfile

# 先验证配置是否合法，再应用
if caddy validate --config /etc/caddy/Caddyfile; then
    # 使用 restart 而非 reload：首次部署时 Caddy 可能还在用默认配置，
    # reload 有时不能正确切换到新域名的 TLS 证书申请
    systemctl restart caddy
    echo "=> Caddy 配置验证通过并已重启"
else
    echo "错误: Caddyfile 验证失败，请手动检查 /etc/caddy/Caddyfile"
    echo "Caddy 仍在使用旧配置运行"
    exit 1
fi

# 等待 Caddy 启动就绪
sleep 2

# ===================== 部署验证 =====================
echo ""
echo "=> 正在验证本地服务..."

# 验证 Python 后端是否响应
if curl -sf -o /dev/null "http://127.0.0.1:$BACKEND_PORT/sub/$TOKEN.yaml"; then
    echo "   [OK] Python 订阅服务正常响应 (Clash Meta YAML)"
else
    echo "   [WARN] Python 订阅服务未响应 (YAML)，请检查: journalctl -u sub-server -n 40"
fi

if curl -sf -o /dev/null "http://127.0.0.1:$BACKEND_PORT/sub/$TOKEN.json"; then
    echo "   [OK] Python 订阅服务正常响应 (sing-box JSON)"
else
    echo "   [WARN] Python 订阅服务未响应 (JSON)，请检查: journalctl -u sub-server -n 40"
fi

if curl -sf -o /dev/null "http://127.0.0.1:$BACKEND_PORT/sub/$TOKEN.txt"; then
    echo "   [OK] Python 订阅服务正常响应 (Shadowrocket TXT)"
else
    echo "   [WARN] Python 订阅服务未响应 (TXT)，请检查: journalctl -u sub-server -n 40"
fi

# 验证 Caddy 是否正常转发，并等待证书申请完成
echo "=> 正在等待 Caddy 申请 SSL 证书并验证 HTTPS 访问..."
echo "   (这可能需要 5-15 秒，请耐心等待。如果云服务商安全组未放行 80 和 443 端口，将会超时)"

max_attempts=15
caddy_success=false
for ((i=1; i<=max_attempts; i++)); do
    # 设置 3 秒超时，静默模式
    HTTP_CODE=$(curl -m 3 -s -o /dev/null -w "%{http_code}" "https://$DOMAIN/sub/$TOKEN.yaml?token=$TOKEN" || echo "000")
    if [ "$HTTP_CODE" = "200" ]; then
        caddy_success=true
        break
    fi
    sleep 2
done

if [ "$caddy_success" = true ]; then
    echo "   [OK] Caddy HTTPS 转发正常 (SSL 证书申请成功)"
else
    echo ""
    echo -e "\033[31m#########################################################################\033[0m"
    echo -e "\033[31m                        [ 警告: HTTPS 证书申请失败 ]                     \033[0m"
    echo -e "\033[31m#########################################################################\033[0m"
    echo "Caddy 无法为 $DOMAIN 申请到 HTTPS 证书。通常有以下原因："
    echo ""
    echo "  1. ⚠️ 防火墙拦截 (最常见): 您的云服务器 (AWS/阿里云/腾讯云等) 安全组未放行 80 和 443 端口。"
    echo "  2. ⚠️ DNS 未生效: 您的域名还没有正确解析到本机 IP。"
    echo "  3. ⚠️ 内部防火墙: ufw 或 iptables 阻挡了 80 和 443 端口。"
    echo ""
    echo "解决办法："
    echo "  - 请前往云服务商控制台，在【安全组/防火墙】中添加入站规则，允许 TCP 80 和 443 端口。"
    echo "  - 若在系统内使用了 ufw，请执行: ufw allow 80/tcp && ufw allow 443/tcp"
    echo "  - 放行端口后，执行以下命令重启 Caddy，即可自动获取证书并恢复正常: "
    echo ""
    echo -e "\033[32m      systemctl restart caddy\033[0m"
    echo ""
    echo -e "\033[31m#########################################################################\033[0m"
    echo ""
    read -rp ">> 按回车键继续查看订阅链接 (请在修复网络后使用)... "
fi

# ===================== 输出部署信息 =====================
# 有明文密码时 (新密码 或 已保存但用户输入了明文) 编码显示，否则显示占位符
if [ "$SAVED_PASSWORD_MODE" = false ]; then
    ENCODED_USER=$(urlencode "$CADDY_USER")
    ENCODED_PASS=$(urlencode "$CADDY_PASS")
    SHOW_PASSWORD="$CADDY_PASS"
    SHOW_ONE_CLICK=true
else
    ENCODED_USER=$(urlencode "$CADDY_USER")
    ENCODED_PASS="<密码>"
    SHOW_PASSWORD="<已保存的密码，如需查看请重新配置>"
    SHOW_ONE_CLICK=true
fi

# 终端二维码辅助函数 (依赖 python3 qrcode[tty]，失败时静默跳过)
print_qr() {
    local url="$1"
    local encoded_url
    encoded_url=$(urlencode "$url")
    echo "  网页二维码链接: https://api.qrserver.com/v1/create-qr-code/?size=400x400&data=${encoded_url}"
    python3 -c "
import sys
try:
    import qrcode
    qr = qrcode.QRCode(border=1)
    qr.add_data(sys.argv[1])
    qr.make(fit=True)
    qr.print_ascii(invert=True)
except Exception:
    pass
" "$url" 2>/dev/null || true
}

echo ""
echo "=================================================="
echo "                   部署完成!                      "
echo "=================================================="
echo ""
echo "============= Clash Meta (YAML) 订阅 ============="
echo ""
echo "--- 方式一: BasicAuth 认证访问 (Clash Party / Stash) ---"
echo ""
echo "  订阅地址: https://$DOMAIN/sub/$TOKEN.yaml"
echo "  认证方式: Basic Auth"
echo "  用户名:   $CADDY_USER"
echo "  密码:     $SHOW_PASSWORD"
echo ""
if [ "$SHOW_ONE_CLICK" = true ]; then
    if [ "$SAVED_PASSWORD_MODE" = false ]; then
        echo "  一键导入链接 (已自动 URL 编码):"
    else
        echo "  一键导入链接 (请手动替换 <密码>):"
    fi
    echo "  https://${ENCODED_USER}:${ENCODED_PASS}@${DOMAIN}/sub/${TOKEN}.yaml"
    echo ""
    echo "  扫码导入 (BasicAuth):"
    print_qr "https://${ENCODED_USER}:${ENCODED_PASS}@${DOMAIN}/sub/${TOKEN}.yaml"
else
    echo "  一键导入链接:"
    echo "  https://<用户名>:<密码>@${DOMAIN}/sub/${TOKEN}.yaml"
    echo "  (请手动替换 <用户名> 和 <密码>)"
fi
echo ""
echo "--- 方式二: Token 免密访问 (CMFA / 不支持 BasicAuth 的客户端) ---"
echo ""
echo "  https://${DOMAIN}/sub/${TOKEN}.yaml?token=${TOKEN}"
echo ""
echo "  扫码导入 (Token 免密):"
print_qr "https://${DOMAIN}/sub/${TOKEN}.yaml?token=${TOKEN}"
echo ""
echo "============= sing-box (JSON) 订阅 ============="
echo ""
echo "--- 方式一: BasicAuth 认证访问 ---"
echo ""
echo "  订阅地址: https://$DOMAIN/sub/$TOKEN.json"
echo "  认证方式: Basic Auth"
echo "  用户名:   $CADDY_USER"
echo "  密码:     $SHOW_PASSWORD"
echo ""
if [ "$SHOW_ONE_CLICK" = true ]; then
    if [ "$SAVED_PASSWORD_MODE" = false ]; then
        echo "  一键导入链接 (已自动 URL 编码):"
    else
        echo "  一键导入链接 (请手动替换 <密码>):"
    fi
    echo "  https://${ENCODED_USER}:${ENCODED_PASS}@${DOMAIN}/sub/${TOKEN}.json"
    echo ""
    echo "  扫码导入 (BasicAuth):"
    print_qr "https://${ENCODED_USER}:${ENCODED_PASS}@${DOMAIN}/sub/${TOKEN}.json"
else
    echo "  一键导入链接:"
    echo "  https://<用户名>:<密码>@${DOMAIN}/sub/${TOKEN}.json"
    echo "  (请手动替换 <用户名> 和 <密码>)"
fi
echo ""
echo "--- 方式二: Token 免密访问 (SFA / SFI / SFM 等 sing-box 客户端) ---"
echo ""
echo "  https://${DOMAIN}/sub/${TOKEN}.json?token=${TOKEN}"
echo ""
echo "  扫码导入 (Token 免密):"
print_qr "https://${DOMAIN}/sub/${TOKEN}.json?token=${TOKEN}"
echo ""
echo "========== Shadowrocket (TXT) 订阅 =========="
echo ""
echo "--- 方式一: BasicAuth 认证访问 ---"
echo ""
echo "  订阅地址: https://$DOMAIN/sub/$TOKEN.txt"
echo "  认证方式: Basic Auth"
echo "  用户名:   $CADDY_USER"
echo "  密码:     $SHOW_PASSWORD"
echo ""
if [ "$SHOW_ONE_CLICK" = true ]; then
    if [ "$SAVED_PASSWORD_MODE" = false ]; then
        echo "  一键导入链接 (已自动 URL 编码):"
    else
        echo "  一键导入链接 (请手动替换 <密码>):"
    fi
    echo "  https://${ENCODED_USER}:${ENCODED_PASS}@${DOMAIN}/sub/${TOKEN}.txt"
    echo ""
    echo "  扫码导入 (BasicAuth):"
    print_qr "https://${ENCODED_USER}:${ENCODED_PASS}@${DOMAIN}/sub/${TOKEN}.txt"
else
    echo "  一键导入链接:"
    echo "  https://<用户名>:<密码>@${DOMAIN}/sub/${TOKEN}.txt"
    echo "  (请手动替换 <用户名> 和 <密码>)"
fi
echo ""
echo "--- 方式二: Token 免密访问 (推荐 Shadowrocket 使用) ---"
echo ""
echo "  https://${DOMAIN}/sub/${TOKEN}.txt?token=${TOKEN}"
echo ""
echo "  扫码导入 (Token 免密):"
print_qr "https://${DOMAIN}/sub/${TOKEN}.txt?token=${TOKEN}"
echo ""
echo "=================================================="
echo ""
echo "服务状态:"
echo "  systemctl status sub-server caddy"
echo "  systemctl list-timers --all | grep -E 'refresh|reset'"
echo ""
echo "常用排查命令:"
echo "  journalctl -u sub-server -n 80 --no-pager"
echo "  journalctl -u caddy -n 80 --no-pager"
echo ""
echo "测试命令 (Clash Meta YAML - BasicAuth):"
if [ "$SAVED_PASSWORD_MODE" = false ]; then
    echo "  curl -sD - -u '${CADDY_USER}:${CADDY_PASS}' 'https://${DOMAIN}/sub/${TOKEN}.yaml' -o /dev/null | head -20"
else
    echo "  curl -sD - -u '${CADDY_USER}:<密码>' 'https://${DOMAIN}/sub/${TOKEN}.yaml' -o /dev/null | head -20"
fi
echo ""
echo "测试命令 (Clash Meta YAML - Token 免密):"
echo "  curl -sD - 'https://${DOMAIN}/sub/${TOKEN}.yaml?token=${TOKEN}' -o /dev/null | head -20"
echo ""
echo "测试命令 (sing-box JSON - BasicAuth):"
if [ "$SAVED_PASSWORD_MODE" = false ]; then
    echo "  curl -sD - -u '${CADDY_USER}:${CADDY_PASS}' 'https://${DOMAIN}/sub/${TOKEN}.json' -o /dev/null | head -20"
else
    echo "  curl -sD - -u '${CADDY_USER}:<密码>' 'https://${DOMAIN}/sub/${TOKEN}.json' -o /dev/null | head -20"
fi
echo ""
echo "测试命令 (sing-box JSON - Token 免密):"
echo "  curl -sD - 'https://${DOMAIN}/sub/${TOKEN}.json?token=${TOKEN}' -o /dev/null | head -20"
echo ""
echo "测试命令 (Shadowrocket TXT - BasicAuth):"
if [ "$SAVED_PASSWORD_MODE" = false ]; then
    echo "  curl -sD - -u '${CADDY_USER}:${CADDY_PASS}' 'https://${DOMAIN}/sub/${TOKEN}.txt' -o /dev/null | head -20"
else
    echo "  curl -sD - -u '${CADDY_USER}:<密码>' 'https://${DOMAIN}/sub/${TOKEN}.txt' -o /dev/null | head -20"
fi
echo ""
echo "测试命令 (Shadowrocket TXT - Token 免密):"
echo "  curl -sD - 'https://${DOMAIN}/sub/${TOKEN}.txt?token=${TOKEN}' -o /dev/null | head -20"
echo ""
