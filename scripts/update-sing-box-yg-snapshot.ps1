param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string]$Commit
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$vendor = Join-Path $repoRoot 'third_party/sing-box-yg'
$temp = Join-Path ([System.IO.Path]::GetTempPath()) "sing-box-yg-$Commit"

try {
    git clone https://github.com/yonggekkk/sing-box-yg.git $temp
    git -C $temp checkout $Commit

    foreach ($file in @('LICENSE', 'README.md', 'sb.sh', 'sb.txt', 'version')) {
        Copy-Item -LiteralPath (Join-Path $temp $file) -Destination (Join-Path $vendor $file) -Force
    }

    $metadata = @"
# sing-box-yg 固定快照

- 上游仓库：<https://github.com/yonggekkk/sing-box-yg>
- 固定 commit：**$Commit**
- 更新日期：$(Get-Date -Format 'yyyy-MM-dd')
- 许可证：GPL-3.0（完整文本见 `LICENSE`）

更新后必须将 vps-sub-meter.sh 中的 UPSTREAM_SINGBOX_YG_COMMIT 更新为同一 commit，
并重新验证 /etc/s-box/clmi.yaml、sbox.json 和 jhsub.txt。
"@
    Set-Content -LiteralPath (Join-Path $vendor 'UPSTREAM.md') -Value $metadata -NoNewline
}
finally {
    if (Test-Path -LiteralPath $temp) {
        Remove-Item -LiteralPath $temp -Recurse -Force
    }
}
