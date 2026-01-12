param(
    [string]$Version = "",
    [switch]$Yes,
    [string]$Prefix = "",
    [string]$BaseUrl = ""
)

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

function Get-Home {
    if (-not [string]::IsNullOrWhiteSpace($env:HOME)) {
        return $env:HOME
    }
    if (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) {
        return $env:USERPROFILE
    }
    [Environment]::GetFolderPath("UserProfile")
}

function Resolve-Prefix([string]$Value) {
    if ([string]::IsNullOrWhiteSpace($Value)) {
        $Value = Join-Path (Get-Home) ".local\bin"
    }
    elseif ($Value.StartsWith("~")) {
        $Value = Join-Path (Get-Home) $Value.Substring(2)
    }
    [IO.Path]::GetFullPath($Value)
}

function Detect-Arch {
    switch ($env:PROCESSOR_ARCHITECTURE.ToLower()) {
        "amd64" { "amd64" }
        "arm64" { "arm64" }
        default { throw "Unsupported arch: $($env:PROCESSOR_ARCHITECTURE). Supported: amd64, arm64." }
    }
}

function Get-BaseUrl([string]$Value) {
    if (-not [string]::IsNullOrWhiteSpace($Value)) {
        return $Value.TrimEnd("/")
    }
    if (-not [string]::IsNullOrWhiteSpace($env:SSHPOD_BASE_URL)) {
        return $env:SSHPOD_BASE_URL.TrimEnd("/")
    }
    "https://github.com/imos/sshpod/releases/download"
}

function Get-Version([string]$Value) {
    if (-not [string]::IsNullOrWhiteSpace($Value)) {
        return $Value
    }
    $headers = @{ "User-Agent" = "sshpod-install" }
    if ($env:GITHUB_TOKEN) {
        $headers["Authorization"] = "Bearer $($env:GITHUB_TOKEN)"
    }
    $version = ""
    foreach ($attempt in 1..5) {
        try {
            $resp = Invoke-WebRequest -UseBasicParsing -Headers $headers -Uri "https://api.github.com/repos/imos/sshpod/releases/latest"
            $json = $resp.Content | ConvertFrom-Json
            $version = $json.tag_name.TrimStart("v")
            if (-not [string]::IsNullOrWhiteSpace($version)) { break }
        }
        catch {
            Start-Sleep -Seconds $attempt
        }
    }
    if ([string]::IsNullOrWhiteSpace($version)) {
        throw "Failed to determine latest version from GitHub releases after retries."
    }
    return $version
}

function Prompt-Configure([string]$ExePath) {
    if ($Yes) {
        & $ExePath configure
        return
    }
    $ans = Read-Host "Run sshpod configure to update ~/.ssh/config now? [y/N]"
    if ($ans -match "^[yY]$") {
        & $ExePath configure
    }
    else {
        Write-Host "Skipping ssh config update."
    }
}

function Main {
    $prefix = Resolve-Prefix $Prefix
    $version = Get-Version $Version
    $arch = Detect-Arch
    $baseUrl = Get-BaseUrl $BaseUrl
    $candidates = @(
        @{ Name = "sshpod_${version}_windows_${arch}.zip"; Kind = "zip" },
        @{ Name = "sshpod_${version}_windows_${arch}.tar.gz"; Kind = "tar" }
    )

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("sshpod-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    $binDir = Join-Path $tmp "bin"
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null

    try {
        $downloaded = $false
        $assetPath = $null
        $assetKind = $null
        $errors = @()
        foreach ($candidate in $candidates) {
            $assetPath = Join-Path $tmp $candidate.Name
            $url = "$baseUrl/v${version}/$($candidate.Name)"
            Write-Host "Downloading $($candidate.Name) ..."
            try {
                Invoke-WebRequest -UseBasicParsing -Headers @{ "User-Agent" = "sshpod-install" } -Uri $url -OutFile $assetPath
                $assetKind = $candidate.Kind
                $downloaded = $true
                break
            }
            catch {
                $errors += "$($candidate.Name): $($_.Exception.Message)"
            }
        }
        if (-not $downloaded) {
            throw "Failed to download release asset for v${version}. Attempts: $($errors -join '; ')"
        }

        if ($assetKind -eq "zip") {
            Expand-Archive -Path $assetPath -DestinationPath $binDir -Force
        }
        else {
            if (-not (Get-Command "tar" -ErrorAction SilentlyContinue)) {
                throw "tar not found to extract $assetPath"
            }
            & tar -xzf $assetPath -C $binDir
            if ($LASTEXITCODE -ne 0) {
                throw "tar extraction failed for $assetPath"
            }
        }
        $exe = Get-ChildItem -Path $binDir -Filter "sshpod.exe" -Recurse | Select-Object -First 1
        if (-not $exe) {
            throw "sshpod.exe not found in downloaded archive"
        }

        if (-not (Test-Path $prefix)) {
            New-Item -ItemType Directory -Path $prefix -Force | Out-Null
        }
        $dest = Join-Path $prefix "sshpod.exe"
        Copy-Item $exe.FullName $dest -Force
        Write-Host "Installed to $dest"

        if (-not (Get-Command "sshpod.exe" -ErrorAction SilentlyContinue)) {
            Write-Warning "Add $prefix to your PATH to run sshpod.exe"
        }

        Prompt-Configure $dest
    }
    finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Main
