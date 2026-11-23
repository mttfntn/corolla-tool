# Ctrl+drags one file out of a real Explorer window onto the wheel, then reports
# whether the source survived. Takes over the mouse for a few seconds.
# Use a throwaway folder: a missed drop lands back in that same folder.
param(
    [Parameter(Mandatory)][string]$Folder,
    [Parameter(Mandatory)][string]$FileName,
    [switch]$NoCtrl,
    [switch]$ReleaseCtrlBeforeDrop,
    [string]$MidShot = "",
    [double]$Angle = 0,
    [double]$Radius = 150
)

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Threading;
public class Input {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, UIntPtr e);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int t, uint f);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetDpiForSystem();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    public struct RECT { public int L, T, R, B; }
    public static void Glide(int sx, int sy, int tx, int ty, int steps) {
        for (int i = 1; i <= steps; i++) { SetCursorPos(sx + (tx - sx) * i / steps, sy + (ty - sy) * i / steps); Thread.Sleep(15); }
    }
}
"@
[Input]::SetProcessDPIAware() | Out-Null

$source = Join-Path $Folder $FileName
$shell = New-Object -ComObject Shell.Application
explorer.exe $Folder
$window = $null
for ($i = 0; $i -lt 40 -and -not $window; $i++) {
    Start-Sleep -Milliseconds 250
    $window = $shell.Windows() | Where-Object {
        try { $_.Document.Folder.Self.Path -eq (Resolve-Path $Folder).Path } catch { $false }
    } | Select-Object -First 1
}
if (-not $window) { throw "Explorer window not found" }

$ctrl = -not $NoCtrl
try {
    $hwnd = [IntPtr]$window.HWND
    # Topmost and placed on the primary screen, so nothing is behind the gesture but itself.
    [Input]::SetWindowPos($hwnd, [IntPtr](-1), 100, 60, 1500, 1100, 0x40) | Out-Null
    [Input]::SetForegroundWindow($hwnd) | Out-Null
    Start-Sleep -Milliseconds 900

    $root = [Windows.Automation.AutomationElement]::FromHandle($hwnd)
    $condition = New-Object Windows.Automation.PropertyCondition(
        [Windows.Automation.AutomationElement]::NameProperty, $FileName)
    $item = $root.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
    if (-not $item) { throw "File item not found in Explorer" }
    $box = $item.Current.BoundingRectangle
    $x = [int]($box.X + $box.Width / 2)
    $y = [int]($box.Y + $box.Height / 2)
    "dragging from $x,$y"

    [Input]::SetCursorPos($x, $y) | Out-Null
    Start-Sleep -Milliseconds 200
    if ($ctrl) { [Input]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 50 }
    [Input]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    [Input]::Glide($x, $y, $x + 40, $y + 40, 8)
    $x += 40; $y += 40
    Start-Sleep -Milliseconds 300
    [Input]::Glide($x, $y, $x + 8, $y + 8, 4)
    $x += 8; $y += 8
    Start-Sleep -Milliseconds 300
    if ($MidShot) {
        Add-Type -AssemblyName System.Drawing
        $bmp = New-Object Drawing.Bitmap 1700, 1250
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen(0, 0, 0, 0, $bmp.Size)
        $bmp.Save($MidShot)
        $g.Dispose(); $bmp.Dispose()
    }

    $cx = $x; $cy = $y
    $wheel = [Input]::FindWindow($null, "Corolla")
    $rect = New-Object Input+RECT
    if ($wheel -ne [IntPtr]::Zero -and [Input]::IsWindowVisible($wheel) -and [Input]::GetWindowRect($wheel, [ref]$rect)) {
        $cx = [int](($rect.L + $rect.R) / 2); $cy = [int](($rect.T + $rect.B) / 2)
        "wheel visible, centre $cx,$cy"
    } else { "wheel not visible" }

    $scale = [Input]::GetDpiForSystem() / 96.0
    $a = $Angle * [Math]::PI / 180
    [Input]::Glide($x, $y, $cx + [int]($Radius * $scale * [Math]::Sin($a)), $cy - [int]($Radius * $scale * [Math]::Cos($a)), 20)
    Start-Sleep -Milliseconds 300
    if ($ReleaseCtrlBeforeDrop) {
        [Input]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 300
        # OLE re-reads key state on movement, so make sure it notices.
        $p = [Windows.Forms.Cursor]::Position
        [Input]::Glide($p.X, $p.Y, $p.X + 6, $p.Y, 3)
        Start-Sleep -Milliseconds 200
    }
}
finally {
    [Input]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
    if ($ctrl) { [Input]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero) }
    Start-Sleep -Milliseconds 800
    $window.Quit()
}
Start-Sleep -Seconds 2
"source still exists: " + (Test-Path $source)
"folder now holds: " + ((Get-ChildItem $Folder | ForEach-Object Name) -join ", ")
