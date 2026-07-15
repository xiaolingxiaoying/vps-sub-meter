<#
.SYNOPSIS
Refreshes the vendored yonggekkk/warp-yg snapshot at an explicit commit.

.DESCRIPTION
This is the only supported upgrade path for the standard WARP dependency.
It intentionally never accepts a branch name. Review the resulting diff,
the upstream license state, and the new SHA-256 before committing.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$Commit,

    [string]$Repository = 'https://github.com/yonggekkk/warp-yg.git'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$target = Join-Path $root 'third_party/warp-yg'
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ('vps-sub-meter-warp-yg-' + [guid]::NewGuid())

try {
    git clone --filter=blob:none --no-checkout $Repository $temp
    git -C $temp fetch --depth=1 origin $Commit
    git -C $temp checkout --detach $Commit

    foreach ($name in @('CFwarp.sh', 'README.md', 'version')) {
        $source = Join-Path $temp $name
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Pinned commit does not contain required file: $name"
        }
        Copy-Item -LiteralPath $source -Destination (Join-Path $target $name) -Force
    }

    $hash = (Get-FileHash -LiteralPath (Join-Path $target 'CFwarp.sh') -Algorithm SHA256).Hash.ToLowerInvariant()
    $licenseNote = if (Test-Path -LiteralPath (Join-Path $temp 'LICENSE')) {
        Copy-Item -LiteralPath (Join-Path $temp 'LICENSE') -Destination (Join-Path $target 'LICENSE') -Force
        'A LICENSE file was copied from the pinned upstream commit.'
    } else {
        Remove-Item -LiteralPath (Join-Path $target 'LICENSE') -Force -ErrorAction SilentlyContinue
        'The pinned upstream commit has no LICENSE file; do not relicense this copy.'
    }

    @(
        '# yonggekkk/warp-yg snapshot'
        ''
        "- Source: $Repository"
        "- Pinned commit: $Commit"
        '- Imported files: CFwarp.sh, version, and the upstream README.md.'
        "- Integrity: CFwarp.sh SHA-256 is $hash."
        ''
        $licenseNote
        'Review this source and its license status before committing the snapshot.'
    ) | Set-Content -LiteralPath (Join-Path $target 'UPSTREAM.md') -Encoding utf8

    $scriptPath = Join-Path $root 'vps-sub-meter.sh'
    $text = Get-Content -LiteralPath $scriptPath -Raw
    $text = $text -replace 'UPSTREAM_WARP_YG_COMMIT="[0-9a-f]{40}"', ('UPSTREAM_WARP_YG_COMMIT="' + $Commit + '"')
    $text = $text -replace 'UPSTREAM_WARP_YG_SHA256="[0-9a-f]{64}"', ('UPSTREAM_WARP_YG_SHA256="' + $hash + '"')
    Set-Content -LiteralPath $scriptPath -Value $text -Encoding utf8NoBOM
    Write-Output "Updated warp-yg snapshot to $Commit with SHA-256 $hash"
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
