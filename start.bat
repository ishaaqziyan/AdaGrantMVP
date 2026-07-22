@echo off
setlocal enabledelayedexpansion
cd /d "%~dp0"

echo Checking Docker...
docker --version >nul 2>&1
if errorlevel 1 (
    echo.
    echo Docker is not installed.
    echo Download and install Docker Desktop from:
    echo https://www.docker.com/products/docker-desktop/
    echo Then run this file again.
    echo.
    pause
    exit /b 1
)

docker info >nul 2>&1
if errorlevel 1 (
    echo.
    echo Docker Desktop is installed but not running.
    echo Open Docker Desktop from the Start menu, wait until it says
    echo "Docker Desktop is running", then run this file again.
    echo.
    pause
    exit /b 1
)

if not exist "offchain\.env" (
    echo.
    echo First-time setup: creating offchain\.env
    copy "offchain\.env.example" "offchain\.env" >nul
    echo.
    echo IMPORTANT: open offchain\.env in Notepad and replace
    echo BLOCKFROST_PROJECT_ID with your real Blockfrost project ID.
    echo Get one free at https://blockfrost.io
    echo.
    echo Once you've saved that file, run this file again.
    echo.
    pause
    exit /b 0
)

echo.
echo Starting the app (this can take a few minutes the first time)...
echo Frontend will be at http://localhost:4321
echo Press Ctrl+C to stop.
echo.
docker compose up --build

pause
