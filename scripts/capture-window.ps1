param(
    [string]$Out = "docs\screenshot.png",
    [int]$ProcessId = 0
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class SwiftSubCapture {
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attr, out RECT pvAttribute, int cbAttribute);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, uint nFlags);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

[SwiftSubCapture]::SetProcessDPIAware() | Out-Null
$process = if ($ProcessId -gt 0) {
    Get-Process -Id $ProcessId -ErrorAction Stop
} else {
    Get-Process swiftsub -ErrorAction Stop | Select-Object -First 1
}
$hwnd = $process.MainWindowHandle
if ($hwnd -eq [IntPtr]::Zero) { throw "SwiftSub window not found" }

# Keep documentation screenshots at the app's default 1180x760 logical size (150% DPI).
[SwiftSubCapture]::ShowWindow($hwnd, 9) | Out-Null
[SwiftSubCapture]::SetWindowPos($hwnd, [IntPtr]::Zero, 80, 80, 1774, 1142, 0x0050) | Out-Null
Start-Sleep -Milliseconds 500

$winRect = New-Object SwiftSubCapture+RECT
[SwiftSubCapture]::GetWindowRect($hwnd, [ref]$winRect) | Out-Null
$frameRect = New-Object SwiftSubCapture+RECT
$dwm = [SwiftSubCapture]::DwmGetWindowAttribute($hwnd, 9, [ref]$frameRect, 16)
if ($dwm -ne 0) { $frameRect = $winRect }

$fullW = $winRect.Right - $winRect.Left
$fullH = $winRect.Bottom - $winRect.Top
$cropX = [Math]::Max(0, $frameRect.Left - $winRect.Left)
$cropY = [Math]::Max(0, $frameRect.Top - $winRect.Top)
$cropW = [Math]::Min($fullW - $cropX, $frameRect.Right - $frameRect.Left)
$cropH = [Math]::Min($fullH - $cropY, $frameRect.Bottom - $frameRect.Top)

Add-Type -AssemblyName System.Drawing
$fullBmp = New-Object System.Drawing.Bitmap $fullW, $fullH
$graphics = [System.Drawing.Graphics]::FromImage($fullBmp)
$hdc = $graphics.GetHdc()
$ok = [SwiftSubCapture]::PrintWindow($hwnd, $hdc, 2)
$graphics.ReleaseHdc($hdc)
$graphics.Dispose()

$cropped = $fullBmp.Clone((New-Object System.Drawing.Rectangle($cropX, $cropY, $cropW, $cropH)), $fullBmp.PixelFormat)
$fullBmp.Dispose()

$resolved = [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $Out))
$cropped.Save($resolved, [System.Drawing.Imaging.ImageFormat]::Png)
$cropped.Dispose()
Write-Output "$resolved ($cropW x $cropH, ok=$ok)"
