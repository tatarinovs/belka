@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"

REM The app version lives in two files that must stay in sync:
REM   Cargo.toml        [workspace.package] - CLI, crates, GUI backend (CARGO_PKG_VERSION)
REM   gui\package.json  - installers, Android versionName/versionCode, getVersion()

set "NEW_VERSION=%~1"
if "%NEW_VERSION%"=="" goto SHOW
if /i "%NEW_VERSION%"=="-h" goto USAGE
if /i "%NEW_VERSION%"=="--help" goto USAGE

powershell -NoProfile -Command "if ($env:NEW_VERSION -notmatch '^\d+\.\d+\.\d+$') { exit 1 }"
if errorlevel 1 goto ERR_FORMAT

powershell -NoProfile -Command "$re = [regex]'(?s)(\[workspace\.package\].*?\nversion\s*=\s*\x22)([^\x22]*)'; $t = [IO.File]::ReadAllText('Cargo.toml'); if (-not $re.IsMatch($t)) { exit 1 }; $n = $re.Replace($t, '${1}' + $env:NEW_VERSION, 1); [IO.File]::WriteAllText('Cargo.toml', $n, [Text.UTF8Encoding]::new($false))"
if errorlevel 1 goto ERR_CARGO

where npm >nul 2>nul
if errorlevel 1 goto ERR_NO_NPM
pushd gui
call npm version %NEW_VERSION% --no-git-tag-version --allow-same-version >nul
set NPM_ERR=%errorlevel%
popd
if not "%NPM_ERR%"=="0" goto ERR_NPM

REM Refresh the workspace crates in Cargo.lock without touching dependencies
cargo update --workspace --quiet
if errorlevel 1 goto ERR_LOCK

echo [OK] Version set to %NEW_VERSION%
echo.
echo Next: commit, then tag the release (the Android workflow builds on v* tags):
echo   git commit -am "Version %NEW_VERSION%"
echo   git tag v%NEW_VERSION%
echo.

:SHOW
powershell -NoProfile -Command "$re = [regex]'(?s)(\[workspace\.package\].*?\nversion\s*=\s*\x22)([^\x22]*)'; $cargo = $re.Match([IO.File]::ReadAllText('Cargo.toml')).Groups[2].Value; $npm = ([IO.File]::ReadAllText('gui\package.json') | ConvertFrom-Json).version; Write-Output ('Cargo.toml        ' + $cargo); Write-Output ('gui\package.json  ' + $npm); if ($cargo -ne $npm) { Write-Output ''; Write-Output '[!] Versions differ: run version.bat X.Y.Z to sync them'; exit 1 }"
exit /b %errorlevel%

:USAGE
echo Usage: version.bat          show the current version
echo        version.bat X.Y.Z    set the version in Cargo.toml and gui\package.json
exit /b 0

:ERR_FORMAT
echo [ERROR] Version must look like X.Y.Z, got "%NEW_VERSION%"
exit /b 1

:ERR_CARGO
echo [ERROR] [workspace.package] version not found in Cargo.toml
exit /b 1

:ERR_NO_NPM
echo [ERROR] npm not found: gui\package.json was not updated
exit /b 1

:ERR_NPM
echo [ERROR] npm version failed: gui\package.json was not updated
exit /b 1

:ERR_LOCK
echo [ERROR] cargo update failed: Cargo.lock may be stale
exit /b 1
