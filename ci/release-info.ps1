# Works out what a CI run is for, from the git ref that triggered it.
#
#   is_release  true for a tag "v" + semantic version (v1.2.3, v1.2.3-rc.1); those get a GitHub release
#   prerelease  true when that version has a pre-release part (-rc.1, -beta, ...)
#   version     the version in Cargo.toml, which a release tag has to match
#   artifact    a name for the uploaded build that is safe as a file name
#
# Prints the values as key=value lines, and appends them to $GITHUB_OUTPUT when run by GitHub Actions.
param(
    [Parameter(Mandatory)] [string] $Ref,
    [string] $Manifest = (Join-Path (Split-Path -Parent $PSScriptRoot) 'Cargo.toml')
)
$ErrorActionPreference = 'Stop'

$name = $Ref -replace '^refs/(tags|heads)/', ''
$isTag = $Ref -like 'refs/tags/*'

# The first `version = "..."` line in the manifest is the package's own.
$versionLine = Get-Content $Manifest | Where-Object { $_ -match '^\s*version\s*=\s*"([^"]+)"' } | Select-Object -First 1
if (-not $versionLine) { throw "no version found in $Manifest" }
$null = $versionLine -match '"([^"]+)"'
$version = $Matches[1]

# https://semver.org: MAJOR.MINOR.PATCH, optional -prerelease, optional +build.
$number = '(0|[1-9]\d*)'
$semver = "^v(?<core>$number\.$number\.$number)(-(?<pre>[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*))?(\+(?<build>[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*))?$"

$isRelease = $false
$prerelease = $false
if ($isTag -and $name -cmatch $semver) {
    $isRelease = $true
    $prerelease = [bool] $Matches['pre']
    # Build metadata does not change which version it is.
    $tagVersion = ($name.Substring(1) -split '\+')[0]
    if ($tagVersion -ne $version) {
        throw "tag $name does not match Cargo.toml, which says $version. Set the version in Cargo.toml, commit, and tag that commit."
    }
}

$outputs = [ordered]@{
    is_release = "$isRelease".ToLower()
    prerelease = "$prerelease".ToLower()
    version    = $version
    artifact   = 'coh3-display-' + ($name -replace '[^0-9A-Za-z._-]', '-')
}
foreach ($key in $outputs.Keys) {
    $line = "$key=$($outputs[$key])"
    $line
    if ($env:GITHUB_OUTPUT) { Add-Content -Path $env:GITHUB_OUTPUT -Value $line -Encoding utf8 }
}
