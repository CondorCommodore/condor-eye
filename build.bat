@echo off
echo Setting up Visual Studio environment...
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1

echo.
echo Running cargo test...
pushd src-tauri
cargo test
if %ERRORLEVEL% EQU 0 (
    echo.
    echo ALL TESTS PASSED
) else (
    echo.
    echo TESTS FAILED - see errors above
)
popd
echo.
pause
