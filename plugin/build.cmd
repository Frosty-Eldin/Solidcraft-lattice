@echo off
rem Builds dist\lattice.zip: the SolidCraft plugin package (plugin.json + lattice.wasm component).
rem One-time setup:
rem   rustup target add wasm32-unknown-unknown
rem   cargo install wasm-tools
rem Tests are separate (they take a few minutes): cargo test --release
setlocal
cd /d "%~dp0"

echo Building the plugin (the first build takes a few minutes)...
cargo build --release --target wasm32-unknown-unknown || goto :fail

if exist dist rmdir /s /q dist
mkdir dist\lattice || goto :fail
wasm-tools component new target\wasm32-unknown-unknown\release\solidcraft_lattice.wasm -o dist\lattice\lattice.wasm || goto :fail
wasm-tools validate dist\lattice\lattice.wasm || goto :fail
copy /y plugin.json dist\lattice\plugin.json >nul || goto :fail

powershell -NoProfile -Command "Compress-Archive -Path 'dist\lattice' -DestinationPath 'dist\lattice.zip' -Force" || goto :fail
echo.
echo Built %CD%\dist\lattice.zip
echo Install: SolidCraft ^> Preferences ^> Plugins ^> Install plugin... ^> pick dist\lattice.zip
exit /b 0

:fail
echo.
echo BUILD FAILED
exit /b 1
