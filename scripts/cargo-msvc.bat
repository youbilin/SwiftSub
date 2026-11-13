@echo off
setlocal enabledelayedexpansion

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VSDIR="

if exist "%VSWHERE%" (
    for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -products * -property installationPath`) do set "VSDIR=%%i"
)

if not defined VSDIR if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\18\BuildTools" set "VSDIR=%ProgramFiles(x86)%\Microsoft Visual Studio\18\BuildTools"
if not defined VSDIR if exist "%ProgramFiles%\Microsoft Visual Studio\2022\BuildTools" set "VSDIR=%ProgramFiles%\Microsoft Visual Studio\2022\BuildTools"
if not defined VSDIR if exist "%ProgramFiles%\Microsoft Visual Studio\2022\Community" set "VSDIR=%ProgramFiles%\Microsoft Visual Studio\2022\Community"
if not defined VSDIR if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools" set "VSDIR=%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools"

if not defined VSDIR (
    echo Visual Studio Build Tools not found.
    echo Install "Desktop development with C++" or run cargo from a Developer Command Prompt.
    exit /b 1
)

call "!VSDIR!\VC\Auxiliary\Build\vcvars64.bat" >nul
if errorlevel 1 exit /b 1

cargo %*
