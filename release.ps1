<#
.SYNOPSIS
    Build the Windows release binary and publish it as a GitHub release.

.DESCRIPTION
    Builds target\release\autowording.exe through build.bat (the GNU toolchain
    wrapper), copies it to dist\AutoWording-v<version>-windows-x64.exe, and
    publishes a GitHub release tagged v<version>. Publishing uses the REST API
    when GH_TOKEN or GITHUB_TOKEN is set, otherwise the authenticated GitHub CLI
    (gh).

    The current commit must already be pushed, because the release tag points at
    it. dist\ is gitignored.

.PARAMETER Version
    Release version without the leading "v". Defaults to Cargo.toml.

.PARAMETER NotesFile
    Markdown file with the release notes. Defaults to a bullet list of the
    commits since the previous tag.

.PARAMETER NoBuild
    Reuse an existing target\release\autowording.exe.

.PARAMETER DryRun
    Print the tag, asset and notes without publishing.

.EXAMPLE
    release.bat
    release.bat 0.1.1
    release.ps1 -Version 0.1.1 -NotesFile notes.md
    release.ps1 -DryRun
#>
[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$NotesFile = "",
    [switch]$NoBuild,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
Push-Location $root
try {
    if (-not $Version) {
        $cargo = Get-Content -Raw -LiteralPath (Join-Path $root "Cargo.toml")
        if ($cargo -notmatch '(?m)^\s*version\s*=\s*"([^"]+)"') {
            throw "Could not read the version from Cargo.toml. Pass one: release.bat 0.1.1"
        }
        $Version = $Matches[1]
    }
    $tag = "v$Version"

    $branch = (& git rev-parse --abbrev-ref HEAD).Trim()
    $head = (& git rev-parse HEAD).Trim()
    $remoteHead = (& git rev-parse --verify --quiet "origin/$branch")
    if ($LASTEXITCODE -ne 0 -or -not $remoteHead -or $remoteHead.Trim() -ne $head) {
        throw "Push $branch first: the $tag tag must point at a commit that exists on GitHub."
    }
    if (& git status --porcelain) {
        Write-Host "warning: uncommitted changes; the binary includes them but the tag and notes describe $head." -ForegroundColor Yellow
    }

    $remote = (& git remote get-url origin).Trim()
    if ($remote -notmatch 'github\.com[:/](?<owner>[^/]+)/(?<repo>[^/]+?)(?:\.git)?$') {
        throw "origin is not a GitHub remote: $remote"
    }
    $owner = $Matches['owner']
    $repo = $Matches['repo']

    if ($NotesFile) {
        $notes = Get-Content -Raw -LiteralPath $NotesFile
    } else {
        $range = "HEAD"
        $previous = (& git for-each-ref --sort=-creatordate --format="%(refname:short)" refs/tags | Select-Object -First 1)
        if ($previous) {
            $range = "$($previous.Trim())..HEAD"
        }
        $commits = @(& git log $range --no-merges --pretty=format:"- %s")
        $notes = "## Changes`n`n" + ($commits -join "`n")
    }

    $exe = Join-Path $root "target\release\autowording.exe"
    if (-not $NoBuild) {
        Write-Host "Building $tag ..."
        & (Join-Path $root "build.bat") build --release
        if ($LASTEXITCODE -ne 0) {
            throw "Build failed with exit code $LASTEXITCODE."
        }
    }
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Missing $exe. Run a release build first or drop -NoBuild."
    }

    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $assetName = "AutoWording-$tag-windows-x64.exe"
    $assetPath = Join-Path $dist $assetName
    Copy-Item -LiteralPath $exe -Destination $assetPath -Force
    $size = "{0:N1} MB" -f ((Get-Item -LiteralPath $assetPath).Length / 1MB)

    if ($DryRun) {
        Write-Host "Tag:    $tag"
        Write-Host "Repo:   $owner/$repo"
        Write-Host "Commit: $head ($branch)"
        Write-Host "Asset:  $assetPath ($size)"
        Write-Host ""
        Write-Host $notes
        return
    }

    $token = $env:GH_TOKEN
    if (-not $token) { $token = $env:GITHUB_TOKEN }
    $gh = Get-Command gh -ErrorAction SilentlyContinue

    if (-not $token -and -not $gh) {
        throw "Set GH_TOKEN or GITHUB_TOKEN with repo access, or install the GitHub CLI (gh) and sign in."
    }

    if ($gh -and -not $token) {
        & gh release create $tag $assetPath --title "AutoWording $tag" --notes $notes --target $head
        if ($LASTEXITCODE -ne 0) {
            throw "gh release create failed with exit code $LASTEXITCODE."
        }
        Write-Host "Published $tag with $assetName ($size)."
        return
    }

    $headers = @{
        Authorization          = "Bearer $token"
        Accept                 = "application/vnd.github+json"
        "X-GitHub-Api-Version" = "2022-11-28"
    }
    $payload = @{
        tag_name         = $tag
        target_commitish = $head
        name             = "AutoWording $tag"
        body             = $notes
        draft            = $false
        prerelease       = $false
    } | ConvertTo-Json -Depth 4

    try {
        $release = Invoke-RestMethod -Method Post -UserAgent "AutoWordingRelease" `
            -Uri "https://api.github.com/repos/$owner/$repo/releases" `
            -Headers $headers -ContentType "application/json" -Body $payload
    }
    catch {
        throw "Could not create release ${tag}. $($_.Exception.Message)"
    }

    $uploadUri = "https://uploads.github.com/repos/$owner/$repo/releases/$($release.id)/assets?name=$([uri]::EscapeDataString($assetName))"
    try {
        Invoke-RestMethod -Method Post -UserAgent "AutoWordingRelease" `
            -Uri $uploadUri -Headers $headers -ContentType "application/octet-stream" `
            -InFile $assetPath | Out-Null
    }
    catch {
        throw "Release $tag exists but the asset upload failed. $($_.Exception.Message)"
    }

    Write-Host "Published $($release.html_url)"
    Write-Host "Asset:   $assetName ($size)"
    Write-Host "Tag:     $tag -> $head"
}
catch {
    Write-Host "release failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
finally {
    Pop-Location
}
