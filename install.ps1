$ErrorActionPreference = "Stop"

$Repo = "sydneyvb-nl/tellur"
$Version = if ($env:TELLUR_VERSION) { $env:TELLUR_VERSION.TrimStart("v") } else { "latest" }
$InstallDir = if ($env:TELLUR_INSTALL_DIR) { $env:TELLUR_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\Tellur" }

if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -notin @("AMD64", "x86_64")) {
    throw "Tellur currently provides a prebuilt Windows x64 binary only."
}

$Base = if ($Version -eq "latest") {
    "https://github.com/$Repo/releases/latest/download"
} else {
    "https://github.com/$Repo/releases/download/v$Version"
}
$Temp = Join-Path ([IO.Path]::GetTempPath()) ("tellur-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $Temp | Out-Null

function Download-Verified([string]$Name) {
    $File = Join-Path $Temp $Name
    $Sidecar = "$File.sha256"
    Invoke-WebRequest -UseBasicParsing "$Base/$Name" -OutFile $File
    Invoke-WebRequest -UseBasicParsing "$Base/$Name.sha256" -OutFile $Sidecar
    $Expected = ((Get-Content -Raw $Sidecar).Trim() -split "\s+")[0].ToLowerInvariant()
    $Actual = (Get-FileHash -Algorithm SHA256 $File).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) { throw "Checksum mismatch for $Name" }
    return $File
}

try {
    Write-Host "Installing Tellur CLI…"
    $Archive = Download-Verified "tellur-windows-x64.zip"
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Expand-Archive -Force $Archive $InstallDir

    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $PathEntries = if ($UserPath) { $UserPath -split ";" } else { @() }
    if ($PathEntries -notcontains $InstallDir) {
        $NewPath = if ($UserPath) { "$($UserPath.TrimEnd(';'));$InstallDir" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
        $env:Path = "$InstallDir;$env:Path"
        Write-Host "Added $InstallDir to your user PATH."
    }

    try {
        $Vsix = Download-Verified "tellur-vscode.vsix"
        $Editors = @{
            "VS Code" = @("code", (Join-Path $env:LOCALAPPDATA "Programs\Microsoft VS Code\bin\code.cmd"))
            "Cursor" = @("cursor", (Join-Path $env:LOCALAPPDATA "Programs\Cursor\resources\app\bin\cursor.cmd"))
            "Windsurf" = @("windsurf", (Join-Path $env:LOCALAPPDATA "Programs\Windsurf\bin\windsurf.cmd"))
        }
        foreach ($Editor in $Editors.GetEnumerator()) {
            $Executable = $Editor.Value | Where-Object {
                (Get-Command $_ -ErrorAction SilentlyContinue) -or (Test-Path $_)
            } | Select-Object -First 1
            if ($Executable) {
                Write-Host "Installing Tellur extension in $($Editor.Key)..."
                & $Executable --install-extension $Vsix --force | Out-Null
            }
        }
    } catch {
        Write-Warning "Editor package could not be installed: $($_.Exception.Message)"
    }

    try {
        $Plugin = Download-Verified "tellur-jetbrains.zip"
        $JetBrainsRoot = Join-Path $env:APPDATA "JetBrains"
        if (Test-Path $JetBrainsRoot) {
            Get-ChildItem $JetBrainsRoot -Directory | ForEach-Object {
                $Plugins = Join-Path $_.FullName "plugins"
                $Target = Join-Path $Plugins "tellur-jetbrains"
                New-Item -ItemType Directory -Force -Path $Plugins | Out-Null
                Remove-Item -Recurse -Force $Target -ErrorAction SilentlyContinue
                Expand-Archive -Force $Plugin $Plugins
            }
            Write-Host "Installed the Tellur plugin for detected JetBrains products."
        }
    } catch {
        Write-Warning "JetBrains package could not be installed: $($_.Exception.Message)"
    }

    Write-Host "Starting Tellur setup…"
    & (Join-Path $InstallDir "tellur.exe") setup
    Write-Host "Tellur is installed. Run 'tellur setup status' to inspect the result."
} finally {
    Remove-Item -Recurse -Force $Temp -ErrorAction SilentlyContinue
}
