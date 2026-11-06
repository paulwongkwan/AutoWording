@echo off
rem Build the Windows release exe and publish it as a GitHub release.
rem Usage: release.bat [version] [-NotesFile notes.md] [-NoBuild] [-DryRun]
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0release.ps1" %*
