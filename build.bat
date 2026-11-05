@echo off
rem Build helper for machines where the MSVC toolchain has no Windows SDK.
rem Uses the GNU Rust toolchain together with the portable linker tools in .tools\gnu-bin.
setlocal
set "PATH=%~dp0.tools\gnu-bin;%PATH%"
cargo +stable-x86_64-pc-windows-gnu %*
