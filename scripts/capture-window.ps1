param(
    [string]$Out = "docs\screenshot.png"
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class SwiftSubCapture {
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attr, out RECT pvAttribute, int cbAttribute);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, uint nFlags);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

[SwiftSubCapture]::SetProcessDPIAware() | Out-Null
$process = Get-Process swiftsub -ErrorAction Stop | Select-Object -First 1
$hwnd = $process.MainWindowHandle
if ($hwnd -eq [IntPtr]::Zero) { throw "SwiftSub window not found" }

# Keep documentation screenshots at the app's default 1180x760 logical size (150% DPI).
[SwiftSubCapture]::ShowWindow($hwnd, 9) | Out-Null
[SwiftSubCapture]::SetWindowPos($hwnd, [IntPtr]::Zero, 80, 80, 1774, 1142, 0x0050) | Out-Null
Start-Sleep -Milliseconds 500

$rect = New-Object SwiftSubCapture+RECT
$dwm = [SwiftSubCapture]::DwmGetWindowAttribute($hwnd, 9, [ref]$rect, 16)
if ($dwm -ne 0) { throw "Failed to read window bounds" }

$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
Add-Type -AssemblyName System.Drawing
$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$hdc = $graphics.GetHdc()
$ok = [SwiftSubCapture]::PrintWindow($hwnd, $hdc, 2)
$graphics.ReleaseHdc($hdc)
$graphics.Dispose()

$resolved = [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $Out))
$bitmap.Save($resolved, [System.Drawing.Imaging.ImageFormat]::Png)
$bitmap.Dispose()
Write-Output "$resolved ($width x $height, ok=$ok)"
