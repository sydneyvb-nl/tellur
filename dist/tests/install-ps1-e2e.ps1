$ErrorActionPreference = "Stop"

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Temp = Join-Path ([IO.Path]::GetTempPath()) ("tellur-installer-test-" + [guid]::NewGuid())
$Assets = Join-Path $Temp "assets"
$Repo = Join-Path $Temp "repo"
$TestHome = Join-Path $Temp "home"
New-Item -ItemType Directory -Force -Path $Assets, $Repo, $TestHome | Out-Null

function Write-Checksum([string]$Path) {
    $Hash = (Get-FileHash -Algorithm SHA256 $Path).Hash.ToLowerInvariant()
    "$Hash  $([IO.Path]::GetFileName($Path))" | Set-Content -Encoding ascii "$Path.sha256"
}

try {
    $Binary = Join-Path $Root "target\debug\tellur.exe"
    Compress-Archive -Path $Binary -DestinationPath (Join-Path $Assets "tellur-windows-x64.zip")
    Write-Checksum (Join-Path $Assets "tellur-windows-x64.zip")

    "test-vsix" | Set-Content -Encoding ascii (Join-Path $Assets "tellur-vscode.vsix")
    Write-Checksum (Join-Path $Assets "tellur-vscode.vsix")

    $PluginRoot = Join-Path $Temp "plugin\tellur-jetbrains\lib"
    New-Item -ItemType Directory -Force -Path $PluginRoot | Out-Null
    "test-plugin" | Set-Content -Encoding ascii (Join-Path $PluginRoot "tellur.jar")
    Compress-Archive -Path (Join-Path $Temp "plugin\tellur-jetbrains") -DestinationPath (Join-Path $Assets "tellur-jetbrains.zip")
    Write-Checksum (Join-Path $Assets "tellur-jetbrains.zip")

    $env:TELLUR_VERSION = "0.1.0"
    $env:TELLUR_INSTALL_DIR = Join-Path $Temp "install"
    $env:HOME = $TestHome
    $env:USERPROFILE = $TestHome
    $env:LOCALAPPDATA = Join-Path $TestHome "AppData\Local"
    $env:APPDATA = Join-Path $TestHome "AppData\Roaming"
    New-Item -ItemType Directory -Force -Path (Join-Path $env:APPDATA "JetBrains\IntelliJIdea2025.1") | Out-Null

    function global:Invoke-WebRequest {
        param(
            [switch]$UseBasicParsing,
            [Parameter(Position = 0)][string]$Uri,
            [string]$OutFile
        )
        Copy-Item (Join-Path $Assets ([IO.Path]::GetFileName($Uri))) $OutFile
    }

    Push-Location $Repo
    git init --quiet
    & (Join-Path $Root "install.ps1")
    Pop-Location

    if (-not (Test-Path (Join-Path $env:TELLUR_INSTALL_DIR "tellur.exe"))) {
        throw "CLI was not installed"
    }
    if (-not (Test-Path (Join-Path $Repo ".tellur\config.yml"))) {
        throw "setup wizard did not initialize the repository"
    }
    if (-not (Test-Path (Join-Path $env:APPDATA "JetBrains\IntelliJIdea2025.1\plugins\tellur-jetbrains"))) {
        throw "JetBrains package was not installed"
    }
} finally {
    Remove-Item function:global:Invoke-WebRequest -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $Temp -ErrorAction SilentlyContinue
}
