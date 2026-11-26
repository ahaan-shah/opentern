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
    # A fresh folder in the temporary directory, or in local app data when
    # the temporary directory has no room.
    $roots = @([IO.Path]::GetTempPath(), $env:LOCALAPPDATA) | Where-Object { $_ }
    $state = @{ tmp = $null; root = 0 }
    function New-Work {
        while ($state.root -lt $roots.Count) {
            $dir = Join-Path $roots[$state.root] ("opentern-" + [guid]::NewGuid().ToString('N'))
            $state.root++
            try { New-Item -ItemType Directory -Path $dir -ErrorAction Stop | Out-Null; return $dir } catch {}
        }
        throw "couldn't make a folder to download into"
    }
    # Downloads $name, giving up on a server that doesn't answer and trying
    # again (four times) instead of waiting for the system's own timeout. A
    # download that can't be saved (no room) moves to the next folder.
    function Get-Release($name) {
        $tries = 0
        while ($true) {
            $out = Join-Path $state.tmp $name
            try {
                Invoke-WebRequest -UseBasicParsing -TimeoutSec 60 -Uri "$base/$name" -OutFile $out
                return $out
            } catch [System.IO.IOException] {
                if ($state.root -ge $roots.Count) { throw "couldn't save ${name}: there's no room on this computer" }
                Write-Host "opentern: no room in $(Split-Path $state.tmp), downloading to $($roots[$state.root]) instead"
                Remove-Item -Recurse -Force $state.tmp -ErrorAction SilentlyContinue
                $state.tmp = New-Work
            } catch {
                $tries++
                $status = $_.Exception.Response.StatusCode.value__
                if ($status -eq 404 -or $tries -ge 4) {
                    throw "couldn't download $name ($($_.Exception.Message))"
                }
                Write-Host "opentern: the connection dropped, trying again"
                Start-Sleep -Seconds $tries
            }
        }
    }

    $state.tmp = New-Work
    try {
        Write-Host "opentern: downloading $asset"
        $file = Get-Release $asset

        # Every release carries SHA256SUMS; never install one that doesn't check out.
        $sums = Get-Release 'SHA256SUMS'
        $line = Get-Content $sums | Where-Object { ($_ -split '\s+')[-1].TrimStart('*') -eq $asset } | Select-Object -First 1
        if (-not $line) { throw "$asset isn't listed in the release's SHA256SUMS" }
        $expected = ($line -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -Algorithm SHA256 $file).Hash.ToLowerInvariant()
        if ($expected -ne $actual) { throw "checksum mismatch for $asset" }
        Write-Host "opentern: checksum ok"

        Write-Host "opentern: installing"
        $setupArgs = '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/CLOSEAPPLICATIONS'
        $p = Start-Process -FilePath $file -ArgumentList $setupArgs -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "the installer exited with code $($p.ExitCode)" }
        Write-Host "opentern: installed. Open it from the Start menu."
    } finally {
        if ($state.tmp) { Remove-Item -Recurse -Force $state.tmp -ErrorAction SilentlyContinue }
    }
}
