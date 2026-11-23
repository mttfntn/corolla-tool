# Drives a real OLE drag with synthetic mouse and Ctrl input, for testing the
# wheel on Windows. It takes over the mouse for a few seconds.
#   -Ctrl:       hold Ctrl during the drag (omit to test a plain drag)
#   -Text:       drag text instead of files
#   -Angle/-Radius: where to drop relative to the wheel centre; Radius 0 = hub,
#                    Angle 0 = top wedge, clockwise in degrees
param(
    [string[]]$Files = @(),
    [switch]$Ctrl,
    [switch]$Text,
    [double]$Angle = 0,
    [double]$Radius = 150,
    [string]$Screenshot = ""
)

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms, System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

public class Driver {
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern void mouse_event(uint f, int x, int y, uint d, UIntPtr e);
    [DllImport("user32.dll")] static extern void keybd_event(byte k, byte s, uint f, UIntPtr e);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern int GetDpiForSystem();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindow(string c, string t);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint f);
    public struct POINT { public int X, Y; }
    struct RECT { public int L, T, R, B; }
    const uint LEFTDOWN = 2, LEFTUP = 4, KEYUP = 2;
    const byte VK_CTRL = 0x11;

    public static void Init() { SetProcessDPIAware(); }

    static string Foreground() {
        var sb = new StringBuilder(200);
        GetWindowText(GetForegroundWindow(), sb, 200);
        return sb.ToString();
    }

    static void Glide(ref int x, ref int y, int tx, int ty, int steps) {
        int sx = x, sy = y;
        for (int i = 1; i <= steps; i++) {
            SetCursorPos(sx + (tx - sx) * i / steps, sy + (ty - sy) * i / steps);
            Thread.Sleep(15);
        }
        x = tx; y = ty;
    }

    static void Shot(string path) {
        var b = SystemInformation.VirtualScreen;
        using (var bmp = new Bitmap(b.Width, b.Height))
        using (var g = Graphics.FromImage(bmp)) {
            g.CopyFromScreen(b.Left, b.Top, 0, 0, b.Size);
            bmp.Save(path);
        }
    }

    public static void Run(string[] files, bool ctrl, bool text, double angle, double radius, string shot) {
        // Big enough to hold the whole gesture: it rejects drops, so a miss can
        // never land in whatever app is open behind the test.
        var area = Screen.PrimaryScreen.WorkingArea;
        var form = new Form { Text = "Corolla drag source", Width = 1500, Height = 1100, StartPosition = FormStartPosition.Manual, Left = area.Left + 100, Top = area.Top + 60, TopMost = true, AllowDrop = false };
        bool dragging = false;
        form.MouseMove += (s, e) => {
            if (e.Button != MouseButtons.Left || dragging) return;
            dragging = true;
            var data = new DataObject();
            if (text) data.SetText("just some text");
            else {
                var list = new System.Collections.Specialized.StringCollection();
                list.AddRange(files);
                data.SetFileDropList(list);
            }
            var result = form.DoDragDrop(data, DragDropEffects.Copy | DragDropEffects.Move);
            Console.WriteLine("drag result: " + result);
        };
        form.Shown += (s, e) => {
            var worker = new Thread(() => {
                try {
                    Thread.Sleep(600);
                    int x = form.Left + 700, y = form.Top + 500;
                    SetCursorPos(x, y);
                    Thread.Sleep(150);
                    if (ctrl) { keybd_event(VK_CTRL, 0, 0, UIntPtr.Zero); Thread.Sleep(50); }
                    mouse_event(LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
                    Thread.Sleep(80);
                    Glide(ref x, ref y, x + 50, y + 50, 8);
                    for (int i = 0; i < 3; i++) {
                        Thread.Sleep(50);
                        var probe = new StringBuilder(200); POINT p; GetCursorPos(out p);
                        IntPtr hit = WindowFromPoint(p);
                        GetWindowText(GetAncestor(hit, 2), probe, 200);
                        RECT wr; IntPtr ww = FindWindow(null, "Corolla");
                        string info = "none";
                        if (ww != IntPtr.Zero && GetWindowRect(ww, out wr)) info = "visible=" + IsWindowVisible(ww) + " rect=" + wr.L + "," + wr.T + "," + wr.R + "," + wr.B;
                        Console.WriteLine("probe " + i + ": cursor " + p.X + "," + p.Y + " under='" + probe + "' wheel " + info);
                    }
                    Glide(ref x, ref y, x + 6, y + 6, 3);
                    Thread.Sleep(100);
                    Glide(ref x, ref y, x + 6, y + 6, 3);
                    Thread.Sleep(500);
                    int cx = x, cy = y;
                    var under = new StringBuilder(200);
                    POINT cur; GetCursorPos(out cur);
                    GetWindowText(GetAncestor(WindowFromPoint(cur), 2), under, 200);
                    Console.WriteLine("cursor " + cur.X + "," + cur.Y + " (expected " + x + "," + y + "), window under it: " + under);
                    RECT r;
                    IntPtr wheel = FindWindow(null, "Corolla");
                    if (wheel != IntPtr.Zero && IsWindowVisible(wheel) && GetWindowRect(wheel, out r)) {
                        cx = (r.L + r.R) / 2; cy = (r.T + r.B) / 2;
                        Console.WriteLine("wheel visible, centre " + cx + "," + cy + ", size " + (r.R - r.L));
                    } else Console.WriteLine("wheel not visible");
                    Console.WriteLine("foreground while wheel open: " + Foreground());
                    if (shot != "") Shot(shot + "-open.png");
                    double a = angle * Math.PI / 180.0;
                    double scale = GetDpiForSystem() / 96.0;
                    Glide(ref x, ref y, cx + (int)(radius * scale * Math.Sin(a)), cy - (int)(radius * scale * Math.Cos(a)), 25);
                    Thread.Sleep(350);
                    if (shot != "") Shot(shot + "-hover.png");
                    mouse_event(LEFTUP, 0, 0, 0, UIntPtr.Zero);
                    if (ctrl) keybd_event(VK_CTRL, 0, KEYUP, UIntPtr.Zero);
                    Thread.Sleep(350);
                    if (shot != "") Shot(shot + "-after.png");
                    Thread.Sleep(1000);
                    if (shot != "") Shot(shot + "-later.png");
                } finally {
                    mouse_event(LEFTUP, 0, 0, 0, UIntPtr.Zero);
                    keybd_event(VK_CTRL, 0, KEYUP, UIntPtr.Zero);
                    form.BeginInvoke(new Action(() => form.Close()));
                }
            });
            worker.IsBackground = true;
            worker.Start();
        };
        Application.Run(form);
    }
}
"@

[Driver]::Init()
[Driver]::Run([string[]]$Files, [bool]$Ctrl, [bool]$Text, $Angle, $Radius, $Screenshot)
