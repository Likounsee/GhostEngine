@echo off
setlocal
cd /d "%~dp0"
if errorlevel 1 (
    echo [GhostEngine] Unable to enter the ghostengine workspace.
    exit /b 1
)

set "ITERATIONS=%~1"
if "%ITERATIONS%"=="" set "ITERATIONS=500"

echo [GhostEngine] Running optimized CPU raster benchmarks...
echo [GhostEngine] Iterations per workload: %ITERATIONS%
cargo run --release -p ghostengine --example cpu_raster_bench -- %ITERATIONS%
set "RESULT=%ERRORLEVEL%"

if not "%RESULT%"=="0" (
    echo.
    echo [GhostEngine] Benchmark failed. Check that Rust and Cargo are installed and available in PATH.
)
exit /b %RESULT%
