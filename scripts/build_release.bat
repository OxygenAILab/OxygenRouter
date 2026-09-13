@echo off
REM OxygenRouter release build — safe order for embedded WebUI
REM GitHub@OxygenAILab | OxygenAILab@StarsailsClover
setlocal
cd /d "%~dp0.."

echo [1/4] Building WebUI (vite)...
pushd crates\oxygenrouter-webui\web
call npx vite build || goto :fail
popd

echo [2/4] Purging stale webui artifacts (include_dir re-embed)...
del /q /f target\release\liboxygenrouter_webui* 2>nul
for /d %%D in (target\release\.fingerprint\oxygenrouter-webui-*) do rmdir /s /q "%%D" 2>nul

echo [3/4] Building Rust (release, embed-webui)...
cargo build --release --features embed-webui || goto :fail

echo [4/4] Deploying binary...
copy /y target\release\oxygenrouter.exe release\oxygenrouter-x86_64-pc-windows-msvc\oxygenrouter.exe >nul
echo DONE.
exit /b 0

:fail
echo BUILD FAILED.
exit /b 1
