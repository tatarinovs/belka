@echo off
chcp 65001 >nul
setlocal

echo ===================================================================
echo             BELKA - RELEASE BUILDER
echo ===================================================================
echo.
echo Usage: build.bat [all^|exe^|gui^|linux^|arm64^|android]
echo.

set TARGET_MODE=%1
if "%TARGET_MODE%"=="" set TARGET_MODE=all

if not exist dist mkdir dist

REM Built-in OAuth apps are compiled into both the CLI and the GUI
if not defined BELKA_GOOGLE_CLIENT_ID echo [!] BELKA_GOOGLE_CLIENT_ID is not set: no built-in Google app in this build
if not defined BELKA_YANDEX_CLIENT_ID echo [!] BELKA_YANDEX_CLIENT_ID is not set: no built-in Yandex app in this build

if "%TARGET_MODE%"=="gui" goto BUILD_GUI
if "%TARGET_MODE%"=="linux" goto BUILD_LINUX
if "%TARGET_MODE%"=="arm64" goto BUILD_LINUX
if "%TARGET_MODE%"=="android" goto BUILD_ANDROID

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
REM Build Linux CLI: static musl binaries for x86_64 and ARM64 via Zig
REM ---------------------------------------------------------------------
:BUILD_LINUX
where zig >nul 2>nul
if errorlevel 1 goto ERR_NO_ZIG

where cargo-zigbuild >nul 2>nul
if errorlevel 1 goto INSTALL_ZIGBUILD
goto LINUX_TOOLS_OK

:INSTALL_ZIGBUILD
echo [*] cargo-zigbuild is not installed. Installing...
cargo install cargo-zigbuild
if errorlevel 1 goto ERR_INSTALL_ZIGBUILD

:LINUX_TOOLS_OK
if "%TARGET_MODE%"=="arm64" goto BUILD_ARM64

echo [*] Checking Rust target x86_64-unknown-linux-musl...
rustup target add x86_64-unknown-linux-musl >nul 2>nul

echo.
echo ===================================================================
echo [LINUX] Building Linux x86_64 (x86_64-unknown-linux-musl via Zig)...
echo ===================================================================
cargo zigbuild --release --target x86_64-unknown-linux-musl
if errorlevel 1 goto ERR_BUILD_LINUX

copy /Y "target\x86_64-unknown-linux-musl\release\belka" "dist\belka-linux-x64" >nul
echo [OK] Linux x86_64 binary ready: dist\belka-linux-x64

if "%TARGET_MODE%"=="linux" goto ALL_DONE

:BUILD_ARM64
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
rem goto ALL_DONE

REM ---------------------------------------------------------------------
REM Build Android APK (arm64), signed with the key from
REM gui\src-tauri\gen\android\keystore.properties.
REM ---------------------------------------------------------------------
:BUILD_ANDROID
echo.
echo ===================================================================
echo [ANDROID] Building signed APK for arm64...
echo ===================================================================
if not defined ANDROID_HOME set "ANDROID_HOME=%LOCALAPPDATA%\Android\Sdk"
if not defined JAVA_HOME set "JAVA_HOME=%ProgramFiles%\Android\Android Studio\jbr"
if defined NDK_HOME goto ANDROID_NDK_OK
for /d %%D in ("%ANDROID_HOME%\ndk\*") do set "NDK_HOME=%%D"
:ANDROID_NDK_OK
if not exist "%NDK_HOME%" goto ERR_NO_NDK
if not exist "gui\src-tauri\gen\android\keystore.properties" echo [!] No keystore.properties: the APK will not be signed

pushd gui
if not exist node_modules call npm ci
call npm run tauri -- android build --apk --split-per-abi --target aarch64
set ANDROID_ERR=%errorlevel%
popd
if not "%ANDROID_ERR%"=="0" goto ERR_BUILD_ANDROID

for /f "usebackq delims=" %%V in (`node -p "require('./gui/package.json').version"`) do set "APP_VERSION=%%V"
copy /Y "gui\src-tauri\gen\android\app\build\outputs\apk\arm64\release\app-arm64-release.apk" "dist\belka-%APP_VERSION%-arm64.apk" >nul
echo [OK] Android package ready: dist\belka-%APP_VERSION%-arm64.apk
goto ALL_DONE

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

:ERR_BUILD_LINUX
echo [ERROR] Linux x86_64 build failed!
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1

:ERR_BUILD_ARM64
echo [ERROR] Linux ARM64 build failed!
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1

:ERR_NO_NDK
echo [ERROR] Android NDK was not found. Install it in Android Studio: SDK Manager, SDK Tools, NDK.
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1

:ERR_BUILD_ANDROID
echo [ERROR] Android build failed!
echo.
if "%NO_PAUSE%"=="" pause
exit /b 1
