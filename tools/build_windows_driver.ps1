param([ValidateSet('Debug','Release')][string]$Configuration = 'Release')
$ErrorActionPreference = 'Stop'
if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw 'The native WDK build requires Windows. macOS can perform the separate Clang header check.'
}
$NeonMixRoot = Split-Path $PSScriptRoot -Parent
$NeonMixProject = Join-Path $NeonMixRoot 'drivers/windows/wavert/NeonMixAudio.vcxproj'
$NeonMixArtifacts = Join-Path $NeonMixRoot "artifacts/windows-driver/$Configuration"
$NeonMixPackages = Join-Path $NeonMixRoot '.local/windows-driver-sdk'
$env:NUGET_PACKAGES = Join-Path $NeonMixRoot '.local/nuget'
$env:DOTNET_CLI_HOME = Join-Path $NeonMixRoot '.local/dotnet'
New-Item -ItemType Directory -Force $NeonMixArtifacts, $env:NUGET_PACKAGES, $env:DOTNET_CLI_HOME | Out-Null
if (!(Test-Path (Join-Path $NeonMixPackages 'Microsoft.Windows.WDK.x64.10.0.26100.6584/build/native/Microsoft.Windows.WDK.x64.props'))) {
    throw 'Run tools/dev.ps1 python tools/prepare_windows_driver.py to restore the locked project-local SDK.'
}
$NeonMixVSWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$NeonMixMSBuild = $null
if (Test-Path $NeonMixVSWhere) {
    # The locked WDK ships VS 2022 (17.0) tasks, not VS 2026 (18.0) tasks.
    $NeonMixMSBuild = & $NeonMixVSWhere -latest -version '[17.0,18.0)' -products '*' -requires Microsoft.Component.MSBuild -find 'MSBuild/**/Bin/amd64/MSBuild.exe' | Select-Object -First 1
}
if (!$NeonMixMSBuild) { throw 'The locked WDK requires an installed Visual Studio 2022 MSBuild/C++ toolchain.' }
Write-Output "WDK MSBuild: $NeonMixMSBuild"
& $NeonMixMSBuild $NeonMixProject "/p:Configuration=$Configuration" /p:Platform=x64 /getProperty:WDKContentRoot,WDKBinRoot,InfToolPath,WindowsSdkDir,WindowsSDKVersionedBinRoot,VCToolsVersion,WindowsTargetPlatformVersion | Set-Content (Join-Path $NeonMixArtifacts 'toolchain.json') -Encoding UTF8
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the resolved project-local WDK toolchain.' }
& (Join-Path $PSScriptRoot 'dev.ps1') $NeonMixMSBuild $NeonMixProject /t:Build "/p:Configuration=$Configuration" /p:Platform=x64 /p:SignMode=Off "/bl:$NeonMixArtifacts/build.binlog" /v:minimal
if ($LASTEXITCODE -ne 0) { throw 'WDK build failed; inspect the project-local binary log.' }
$NeonMixOutput = Join-Path $NeonMixRoot "target/windows-driver/$Configuration"
foreach ($NeonMixExtension in @('sys','inf','pdb','cat')) {
    $NeonMixFile = Get-ChildItem $NeonMixOutput -Recurse -Filter "NeonMixAudio.$NeonMixExtension" | Select-Object -First 1
    if (!$NeonMixFile) { throw "Missing WDK output: NeonMixAudio.$NeonMixExtension" }
    Copy-Item $NeonMixFile.FullName $NeonMixArtifacts -Force
}
Copy-Item (Join-Path $NeonMixRoot 'drivers/windows/wavert/LICENSE-MS-PL') $NeonMixArtifacts -Force
$NeonMixHashes = Get-ChildItem $NeonMixArtifacts -File | ForEach-Object {
    @{ name = $_.Name; sha256 = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}
@{ configuration = $Configuration; architecture = 'x64'; signed = $false; files = $NeonMixHashes } | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $NeonMixArtifacts 'build.json') -Encoding UTF8
Write-Output $NeonMixArtifacts
