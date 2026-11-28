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
# and errors are reported rather than calling `exit`, which would close it.
# The script itself is plain ASCII (Windows PowerShell reads a saved copy in
# the system's code page); the logo's characters are made from their codes.
& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    # A terminal that understands colour codes gets the logo and a drawn
    # progress bar; output to a file or an old console gets plain lines.
    $fancy = $false
    try {
        $fancy = $Host.UI.SupportsVirtualTerminal -and -not [Console]::IsOutputRedirected
    } catch {}
    $e = [char]27
    if ($fancy -and -not $env:NO_COLOR) {
        $bold = "$e[1m"; $dim = "$e[2m"; $red = "$e[31m"; $reset = "$e[0m"; $green = "$e[38;2;34;181;108m"
    } else {
        $bold = ''; $dim = ''; $red = ''; $reset = ''; $green = ''
    }
    $block = [string][char]0x2588
    $filledChar = [string][char]0x25A0
    $leftChar = [string][char]0x00B7
    $tick = [string][char]0x2713

    function Say($text) {
        if ($fancy) { Write-Host "  $text" } else { Write-Host "opentern: $text" }
    }
    function Ok($text) {
        if ($fancy) { Write-Host "  $green$tick$reset $text" } else { Write-Host "opentern: $text" }
    }

    # The app icon's two candles, a shade lighter at the top as on the icon.
    function Show-Banner {
        if (-not $fancy) { return }
        $rows = @(
            '          ##  ',
            '  ##    ######',
            '######  ######',
            '##  ##  ######',
            '##  ##  ######',
            '######  ######',
            '  ##    ######',
            '          ##  '
        )
        Write-Host ''
        for ($i = 0; $i -lt $rows.Count; $i++) {
            $colour = $green
            if ($green) {
                $colour = "$e[38;2;$([int](52 - $i * 45 / 7));$([int](211 - $i * 80 / 7));$([int](130 - $i * 56 / 7))m"
            }
            $text = ''
            if ($i -eq 3) { $text = "      $bold" + "Opentern$reset" }
            if ($i -eq 4) { $text = "      $dim" + "installer$reset" }
            Write-Host ("  $colour" + $rows[$i].Replace('#', $block) + "$reset$text")
        }
        Write-Host ''
    }

    function Format-MB($bytes) {
        return ([math]::Floor($bytes * 10 / 1048576) / 10).ToString('0.0', [Globalization.CultureInfo]::InvariantCulture)
    }

    # One line of the bar: filled squares for what's in, dots for what's
    # left, then the percentage and megabytes.
    function Show-Bar($got, $total) {
        $width = 32
        try { if ([Console]::WindowWidth -lt 70) { $width = [math]::Max(10, [Console]::WindowWidth - 38) } } catch {}
        if ($total -gt 0) {
            $filled = [int][math]::Floor([math]::Min($got, $total) * $width / $total)
            $pct = "$([int][math]::Floor([math]::Min($got, $total) * 100 / $total))%"
            $size = "$(Format-MB $got) / $(Format-MB $total) MB"
        } else {
            $filled = 0; $pct = ''; $size = "$(Format-MB $got) MB"
        }
        $bar = ($filledChar * $filled) + $reset + $dim + ($leftChar * ($width - $filled))
        [Console]::Write("`r$e[K  $green$bar$reset  $($pct.PadLeft(4))  $dim$size$reset")
    }

    $repo = 'ahaan-shah/opentern'
    $version = $null
    if ($env:OPENTERN_VERSION) {
        $version = $env:OPENTERN_VERSION.TrimStart('v')
    } elseif (-not $env:OPENTERN_BASE_URL) {
        # Name the version up front (and download exactly that one) by
        # following GitHub's redirect from /releases/latest to its tag.
        try {
            $req = [Net.HttpWebRequest]::Create("https://github.com/$repo/releases/latest")
            $req.Method = 'HEAD'
            $req.AllowAutoRedirect = $false
            $req.Timeout = 10000
            $res = $req.GetResponse()
            $location = $res.Headers['Location']
            $res.Close()
            if ($location -match '/tag/v([^/]+)$') { $version = $Matches[1] }
        } catch {}
    }
    if ($version) {
        $base = "https://github.com/$repo/releases/download/v$version"
        $shown = "Opentern $version"
    } else {
        $base = "https://github.com/$repo/releases/latest/download"
        $shown = 'Opentern'
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

    # Copies one download to $out, drawing the bar when $bar is set. A server
    # that doesn't answer, or a transfer that stops, is given up on.
    function Save-Url($url, $out, $bar) {
        $req = [Net.HttpWebRequest]::Create($url)
        $req.Timeout = 30000
        $req.ReadWriteTimeout = 30000
        $req.UserAgent = 'opentern-installer'
        $res = $req.GetResponse()
        try {
            $total = $res.ContentLength
            $in = $res.GetResponseStream()
            $file = [IO.File]::Create($out)
            try {
                $buf = New-Object byte[] 65536
                $got = 0
                $drawn = [DateTime]::MinValue
                if ($bar) { [Console]::Write("$e[?25l") }
                while (($n = $in.Read($buf, 0, $buf.Length)) -gt 0) {
                    $file.Write($buf, 0, $n)
                    $got += $n
                    if ($bar -and ([DateTime]::UtcNow - $drawn).TotalMilliseconds -ge 100) {
                        Show-Bar $got $total
                        $drawn = [DateTime]::UtcNow
                    }
                }
                if ($bar) {
                    Show-Bar $got $total
                    [Console]::Write("$e[?25h`n`n")
                }
            } finally {
                $file.Close()
                $in.Close()
            }
        } finally {
            $res.Close()
        }
    }

    # Downloads $name, trying again (four times) when the connection drops.
    # A download that can't be saved (no room) moves to the next folder.
    function Get-Release($name, $bar) {
        $tries = 0
        while ($true) {
            $out = Join-Path $state.tmp $name
            try {
                if (-not $fancy) { Say "downloading $name" }
                Save-Url "$base/$name" $out ($fancy -and $bar)
                return $out
            } catch {
                if ($fancy) { [Console]::Write("`r$e[K$e[?25h") }
                # PowerShell wraps what .NET throws; find the cause.
                $ex = $_.Exception
                $full = $false
                $status = $null
                while ($ex) {
                    if ($ex -is [Net.WebException] -and $ex.Response) { $status = [int]$ex.Response.StatusCode }
                    if ($ex -is [IO.IOException] -and (($ex.HResult -band 0xFFFF) -in 0x27, 0x70)) { $full = $true }
                    if (-not $ex.InnerException) { break }
                    $ex = $ex.InnerException
                }
                if ($full) {
                    if ($state.root -ge $roots.Count) { throw "couldn't save ${name}: there's no room on this computer" }
                    Say "No room in $(Split-Path $state.tmp): downloading to $($roots[$state.root]) instead"
                    Remove-Item -Recurse -Force $state.tmp -ErrorAction SilentlyContinue
                    $state.tmp = New-Work
                    continue
                }
                $tries++
                if ($status -eq 404) { throw "couldn't download ${name}: the release doesn't have it" }
                if ($tries -ge 4) { throw "couldn't download ${name} ($($ex.Message))" }
                Say "The connection dropped, trying again"
                Start-Sleep -Seconds $tries
            }
        }
    }

    try {
        Show-Banner
        if ($fancy) { Write-Host "  ${dim}Installing$reset $bold$shown$reset"; Write-Host '' } else { Say "installing $shown" }
        $state.tmp = New-Work
        $file = Get-Release $asset $true

        # Every release carries SHA256SUMS; never install one that doesn't check out.
        $sums = Get-Release 'SHA256SUMS' $false
        $line = Get-Content $sums | Where-Object { ($_ -split '\s+')[-1].TrimStart('*') -eq $asset } | Select-Object -First 1
        if (-not $line) { throw "$asset isn't listed in the release's SHA256SUMS" }
        $expected = ($line -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -Algorithm SHA256 $file).Hash.ToLowerInvariant()
        if ($expected -ne $actual) { throw "checksum mismatch for $asset" }
        Ok 'Checksum verified'

        $setupArgs = '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/CLOSEAPPLICATIONS'
        $p = Start-Process -FilePath $file -ArgumentList $setupArgs -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "the installer exited with code $($p.ExitCode)" }
        Ok 'Installed'
        if ($fancy) { Write-Host '' }
        Say "Open it from the Start menu"
        if ($fancy) { Write-Host '' }
    } catch {
        if ($fancy) {
            [Console]::Write("`r$e[K$e[?25h")
            Write-Host "  $bold${red}error$reset $($_.Exception.Message)"
        } else {
            # Not a terminal: fail the way a script or CI notices.
            throw "opentern: $($_.Exception.Message)"
        }
    } finally {
        if ($state.tmp) { Remove-Item -Recurse -Force $state.tmp -ErrorAction SilentlyContinue }
    }
}
