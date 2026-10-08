$ErrorActionPreference = 'Stop'
$NeonMixRoot = Split-Path $PSScriptRoot -Parent
$env:CARGO_HOME = Join-Path $NeonMixRoot '.local/cargo'
$env:CARGO_TARGET_DIR = Join-Path $NeonMixRoot 'target'
$env:TEMP = Join-Path $NeonMixRoot '.local/tmp'
$env:TMP = $env:TEMP
$env:TMPDIR = $env:TEMP
$env:PYTHONUTF8 = '1'
$env:PIP_CACHE_DIR = Join-Path $NeonMixRoot '.local/pip'
$env:UV_PYTHON_INSTALL_DIR = Join-Path $NeonMixRoot '.local/python'
$env:UV_CACHE_DIR = Join-Path $NeonMixRoot '.local/uv'
$env:npm_config_cache = Join-Path $NeonMixRoot '.local/npm'
$env:PYTHONPYCACHEPREFIX = Join-Path $NeonMixRoot '.local/pycache'
New-Item -ItemType Directory -Force $env:CARGO_HOME, $env:TEMP, $env:UV_CACHE_DIR | Out-Null
$NeonMixRustc = rustup which --toolchain 1.95.0 rustc
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$env:PATH = "$(Split-Path $NeonMixRustc);$env:PATH"
$NeonMixGStreamerLock = Join-Path $NeonMixRoot '.local/gstreamer-windows/lock.json'
if (Test-Path $NeonMixGStreamerLock) {
    $NeonMixGStreamer = (Get-Content $NeonMixGStreamerLock -Raw | ConvertFrom-Json).prefix
    $env:PATH = "$(Join-Path $NeonMixGStreamer 'bin');$env:PATH"
    $env:PKG_CONFIG = Join-Path $NeonMixGStreamer 'bin/pkg-config.exe'
    $env:PKG_CONFIG_PATH = Join-Path $NeonMixGStreamer 'lib/pkgconfig'
    $env:GSTREAMER_1_0_ROOT_MSVC_X86_64 = "$NeonMixGStreamer/"
    $env:GST_PLUGIN_SYSTEM_PATH_1_0 = Join-Path $NeonMixGStreamer 'lib/gstreamer-1.0'
    $env:GST_PLUGIN_PATH_1_0 = ''
    $env:GST_PLUGIN_PATH = ''
    $env:GST_REGISTRY = Join-Path $NeonMixRoot '.local/gstreamer-windows/registry.bin'
}
Set-Location $NeonMixRoot
if ($args.Count -lt 1) { throw 'Usage: tools/dev.ps1 COMMAND [ARGS...]' }
$NeonMixCommand = $args[0]
$NeonMixArguments = @($args | Select-Object -Skip 1)
& $NeonMixCommand @NeonMixArguments
exit $LASTEXITCODE
