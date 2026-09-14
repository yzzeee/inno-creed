# inno-creed 초보자용 GUI 인스톨러 배포 zip을 만든다 (Windows).
#
# 전제: inno-creed 본체와 installer가 이미 release로 빌드돼 있어야 한다.
#   cargo build --release --bin inno-creed
#   cargo build --release -p installer
#
# exe 안에 exe를 내장하지 않는다 — installer.exe와 payload/를 zip 안에서
# 나란히 두고, installer는 실행 시 자기 옆에서 payload를 찾는다
# (installer/src/payload.rs 참고. 이유는 그 파일 문서 주석 참고 —
# "exe 안에 exe 내장 후 디스크에 풀어씀"이 백신 드로퍼 휴리스틱과 겹친다).

param(
    [string]$OutDir = "dist",
    [ValidateSet('', 'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string]$Target = ''
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $releaseDir = if ($Target) { "target/$Target/release" } else { "target/release" }
    $machines = @()
    foreach ($f in @("$releaseDir/installer.exe", "$releaseDir/inno-creed.exe")) {
        if (-not (Test-Path $f)) {
            throw "$f 가 없습니다. 먼저 'cargo build --release --bin inno-creed' 와 'cargo build --release -p installer' 를 실행하세요."
        }
        # 파일명만 ARM64로 바꾸거나 다른 아키텍처의 payload를 섞지 못하게 PE를 확인한다.
        $reader = [System.IO.BinaryReader]::new([System.IO.File]::OpenRead((Join-Path $root $f)))
        try {
            if ($reader.ReadUInt16() -ne 0x5A4D) { throw "$f is not a PE executable." }
            $reader.BaseStream.Position = 0x3C
            $offset = $reader.ReadInt32()
            $reader.BaseStream.Position = $offset
            if ($reader.ReadUInt32() -ne 0x4550) { throw "$f has no PE signature." }
            $machines += $reader.ReadUInt16()
        }
        finally { $reader.Dispose() }
    }
    if ($machines[0] -ne $machines[1]) { throw 'installer and payload architectures differ.' }
    $arch = switch ($machines[0]) {
        0x8664 { 'x86_64' }
        0xAA64 { 'aarch64' }
        default { throw 'Unsupported Windows PE architecture.' }
    }
    if ($Target -and $Target -ne "$arch-pc-windows-msvc") { throw 'PE architecture does not match Target.' }

    $stage = Join-Path $root ".claude/installer-stage-$([System.Guid]::NewGuid())"
    New-Item -ItemType Directory -Force -Path "$stage/payload/extension/icons" | Out-Null

    Copy-Item "$releaseDir/installer.exe" "$stage/installer.exe"
    Copy-Item "$releaseDir/inno-creed.exe" "$stage/payload/inno-creed.exe"
    Copy-Item "extension/manifest.json" "$stage/payload/extension/manifest.json"
    Copy-Item "extension/background.js" "$stage/payload/extension/background.js"
    Copy-Item "extension/icons/*" "$stage/payload/extension/icons/"

    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $zipPath = Join-Path (Resolve-Path $OutDir) "inno-creed-installer-windows-$arch.zip"
    if (Test-Path $zipPath) { Remove-Item $zipPath }

    # Compress-Archive는 zip 엔트리 이름에 백슬래시를 써서 일부 OS의 unzip이 경고를
    # 띄운다(치명적이진 않지만). macOS/Linux에서도 깔끔하게 풀리도록 .NET
    # System.IO.Compression으로 직접 만들어 엔트리 이름 구분자를 항상 '/'로 고정한다.
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::Open($zipPath, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        Get-ChildItem -Path $stage -Recurse -File | ForEach-Object {
            $relative = $_.FullName.Substring($stage.Length + 1).Replace('\', '/')
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $_.FullName, $relative) | Out-Null
        }
    }
    finally {
        $zip.Dispose()
    }

    Remove-Item -Recurse -Force $stage
    Write-Host "만든 파일: $zipPath"
}
finally {
    Pop-Location
}
