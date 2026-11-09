@echo off
rem Build helper for machines where the MSVC toolchain has no Windows SDK.
rem The complete MinGW installation must come first: its gcc finds cc1.exe in
rem libexec and windres.exe in bin. The partial tools in .tools\gnu-bin are only
rem a fallback and must not shadow that compiler.
setlocal
if not defined MINGW64_BIN set "MINGW64_BIN=%ProgramData%\mingw64\mingw64\bin"
if not exist "%MINGW64_BIN%\gcc.exe" (
    echo error: complete MinGW GCC not found at "%MINGW64_BIN%\gcc.exe".
    echo Set MINGW64_BIN to the MinGW bin directory containing gcc.exe and windres.exe.
    exit /b 1
)
set "PATH=%MINGW64_BIN%;%~dp0.tools\gnu-bin;%PATH%"
set "CC=%MINGW64_BIN%\gcc.exe"
if exist "%MINGW64_BIN%\ar.exe" set "AR=%MINGW64_BIN%\ar.exe"
if exist "%MINGW64_BIN%\windres.exe" set "WINDRES=%MINGW64_BIN%\windres.exe"
cargo +stable-x86_64-pc-windows-gnu %*
