# Opentern installer for Windows.
#
#   irm https://raw.githubusercontent.com/ahaan-shah/opentern/main/install.ps1 | iex
#
# Downloads the latest installer, checks its SHA-256 against the release's
# SHA256SUMS, and installs Opentern for the current user (no admin prompt).
#
# Environment:
#   OPENTERN_VERSION   install a specific version (e.g. 0.1.4) instead of the latest
#   OPENTERN_BASE_URL  download from a mirror instead of GitHub releases
#
# Everything runs inside a script block so nothing leaks into your session,
# and errors are thrown rather than calling `exit`, which would close it.
& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is far faster without the progress bar
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $repo = 'ahaan-shah/opentern'
    if ($env:OPENTERN_VERSION) {
        $base = "https://github.com/$repo/releases/download/v$($env:OPENTERN_VERSION.TrimStart('v'))"
    } else {
        $base = "https://github.com/$repo/releases/latest/download"
    }
    if ($env:OPENTERN_BASE_URL) { $base = $env:OPENTERN_BASE_URL.TrimEnd('/') }

    $asset = 'Opentern-windows-setup.exe'
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("opentern-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        Write-Host "opentern: downloading $asset"
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile (Join-Path $tmp $asset)

        $sums = Join-Path $tmp 'SHA256SUMS'
        $haveSums = $true
        try { Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile $sums } catch { $haveSums = $false }
        if ($haveSums) {
            $line = Get-Content $sums | Where-Object { ($_ -split '\s+')[-1].TrimStart('*') -eq $asset } | Select-Object -First 1
            if ($line) {
                $expected = ($line -split '\s+')[0].ToLowerInvariant()
                $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $asset)).Hash.ToLowerInvariant()
                if ($expected -ne $actual) { throw "checksum mismatch for $asset" }
                Write-Host "opentern: checksum ok"
            }
        }

        Write-Host "opentern: installing"
        $setupArgs = '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/CLOSEAPPLICATIONS'
        $p = Start-Process -FilePath (Join-Path $tmp $asset) -ArgumentList $setupArgs -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "the installer exited with code $($p.ExitCode)" }
        Write-Host "opentern: installed. Open it from the Start menu."
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}
