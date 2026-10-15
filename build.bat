@echo off
chcp 65001 >nul
setlocal

echo ===================================================================
echo             BELKA - RELEASE BUILDER
echo ===================================================================
echo.
echo Usage: build.bat [all^|exe^|gui^|arm64]
echo.

set TARGET_MODE=%1
if "%TARGET_MODE%"=="" set TARGET_MODE=all

if not exist dist mkdir dist

REM Built-in OAuth apps are compiled into both the CLI and the GUI
if not defined BELKA_GOOGLE_CLIENT_ID echo [!] BELKA_GOOGLE_CLIENT_ID is not set: no built-in Google app in this build
if not defined BELKA_YANDEX_CLIENT_ID echo [!] BELKA_YANDEX_CLIENT_ID is not set: no built-in Yandex app in this build

if "%TARGET_MODE%"=="gui" goto BUILD_GUI
if "%TARGET_MODE%"=="arm64" goto BUILD_ARM64

REM ---------------------------------------------------------------------
REM Build Windows x86_64 CLI
REM ---------------------------------------------------------------------
echo.
echo ===================================================================
echo [CLI] Building Windows x86_64 (.exe)...
echo ===================================================================
cargo build --release
if errorlevel 1 goto ERR_BUILD_EXE

copy /Y "target\release\belka.exe" "dist\belka.exe" >nul
echo [OK] Windows binary ready: dist\belka.exe

if "%TARGET_MODE%"=="exe" goto ALL_DONE

REM ---------------------------------------------------------------------
REM Build Windows GUI (Tauri): portable exe and NSIS installer
REM ---------------------------------------------------------------------
:BUILD_GUI
echo.
echo ===================================================================
echo [GUI] Building Windows GUI (portable .exe and installer)...
echo ===================================================================
where npm >nul 2>nul
if errorlevel 1 goto ERR_NO_NPM

if exist "gui\node_modules" goto GUI_DEPS_OK
echo [*] Installing frontend dependencies...
pushd gui
call npm ci
set NPM_ERR=%errorlevel%
popd
if not "%NPM_ERR%"=="0" goto ERR_BUILD_GUI

:GUI_DEPS_OK
pushd gui
call npm run tauri -- build --bundles nsis
set GUI_ERR=%errorlevel%
popd
if not "%GUI_ERR%"=="0" goto ERR_BUILD_GUI

copy /Y "target\release\belka-gui.exe" "dist\belka-gui.exe" >nul
copy /Y "target\release\bundle\nsis\*-setup.exe" "dist\" >nul
echo [OK] GUI ready: dist\belka-gui.exe and installer

if "%TARGET_MODE%"=="gui" goto ALL_DONE

REM ---------------------------------------------------------------------
REM Build Linux ARM64 CLI (aarch64-unknown-linux-musl via Zig)
REM ---------------------------------------------------------------------
:BUILD_ARM64
where zig >nul 2>nul
if errorlevel 1 goto ERR_NO_ZIG

where cargo-zigbuild >nul 2>nul
if errorlevel 1 goto INSTALL_ZIGBUILD
goto CHECK_TARGET

:INSTALL_ZIGBUILD
echo [*] cargo-zigbuild is not installed. Installing...
cargo install cargo-zigbuild
if errorlevel 1 goto ERR_INSTALL_ZIGBUILD

:CHECK_TARGET
echo [*] Checking Rust target aarch64-unknown-linux-musl...
rustup target add aarch64-unknown-linux-musl >nul 2>nul

echo.
echo ===================================================================
echo [ARM64] Building Linux ARM64 (aarch64-unknown-linux-musl via Zig)...
echo ===================================================================
cargo zigbuild --release --target aarch64-unknown-linux-musl
if errorlevel 1 goto ERR_BUILD_ARM64

copy /Y "target\aarch64-unknown-linux-musl\release\belka" "dist\belka-linux-arm64" >nul
echo [OK] Linux ARM64 binary ready: dist\belka-linux-arm64

:ALL_DONE
echo.
echo ===================================================================
echo                      BUILD SUCCEEDED!
echo ===================================================================
echo Artifacts saved in dist/ folder:
echo.
for %%F in (dist\*) do echo   %%F   [%%~zF bytes]
echo ===================================================================
echo.
if "%NO_PAUSE%"=="" pause
exit /b 0

:ERR_NO_ZIG
echo [ERROR] zig was not found in PATH.
echo Please install Zig: winget install zig.zig
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1

:ERR_INSTALL_ZIGBUILD
echo [ERROR] Failed to install cargo-zigbuild.
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1

:ERR_NO_NPM
echo [ERROR] npm was not found in PATH.
echo Please install Node.js: winget install OpenJS.NodeJS.LTS
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1

:ERR_BUILD_EXE
echo [ERROR] Windows EXE build failed!
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1

:ERR_BUILD_GUI
echo [ERROR] GUI build failed!
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1

:ERR_BUILD_ARM64
echo [ERROR] Linux ARM64 build failed!
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1
