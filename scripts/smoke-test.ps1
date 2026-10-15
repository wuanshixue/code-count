# 无头冒烟测试：启动 GUI -> 点「选择文件夹」-> 在原生对话框里输入路径 -> 截图 -> 退出。
# 用来在没有人工干预的情况下验证「窗口能画出来、按钮能点、统计结果能渲染」。
#
#   pwsh -File scripts/smoke-test.ps1 -Target D:\code\reveal.js -Out shot.png
#
# 注意：会短暂操作鼠标（移动+点击），跑之前别在用鼠标。

param(
    [Parameter(Mandatory = $true)][string]$Target,
    [string]$Out = "shot.png",
    [string]$Exe = "target\debug\code-count.exe",
    [int]$LaunchWait = 5,
    [int]$ResultWait = 5
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System; using System.Text; using System.Collections.Generic; using System.Runtime.InteropServices;
public class Smoke {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }
  public static void Click(int x, int y) {
    SetCursorPos(x, y); System.Threading.Thread.Sleep(200);
    mouse_event(0x0002,0,0,0,IntPtr.Zero); System.Threading.Thread.Sleep(90);
    mouse_event(0x0004,0,0,0,IntPtr.Zero);
  }
  public static IntPtr Find(uint pid, string title) {
    IntPtr f = IntPtr.Zero;
    EnumWindows((h,p) => { uint q; GetWindowThreadProcessId(h, out q);
      if (q == pid && IsWindowVisible(h)) { var sb = new StringBuilder(512); GetWindowTextW(h, sb, 512);
        if (sb.ToString() == title) { f = h; return false; } } return true; }, IntPtr.Zero);
    return f;
  }
  public static int[] Rect(IntPtr h) { RECT r; GetWindowRect(h, out r); return new int[]{r.L,r.T,r.R,r.B}; }
}
"@

$mainTitle = "代码行数统计"
$dlgTitle  = "选择要统计的文件夹"

Get-Process -Name code-count -ErrorAction SilentlyContinue | Stop-Process -Force
$p = Start-Process -FilePath $Exe -PassThru
try {
    Start-Sleep -Seconds $LaunchWait
    $p.Refresh()
    if ($p.HasExited) { throw "进程启动后立刻退出，退出码 $($p.ExitCode)" }

    $h = [Smoke]::Find([uint32]$p.Id, $mainTitle)
    if ($h -eq [IntPtr]::Zero) { throw "找不到主窗口『$mainTitle』" }
    [Smoke]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Milliseconds 800
    $r = [Smoke]::Rect($h)
    Write-Host "主窗口 rect = $($r -join ',')"

    # 「选择文件夹」按钮在窗口内大约 (940, 80)。
    $dlg = [IntPtr]::Zero
    for ($i = 1; $i -le 5 -and $dlg -eq [IntPtr]::Zero; $i++) {
        [Smoke]::SetForegroundWindow($h) | Out-Null
        Start-Sleep -Milliseconds 300
        [Smoke]::Click($r[0] + 940, $r[1] + 80)
        Start-Sleep -Seconds 2
        $dlg = [Smoke]::Find([uint32]$p.Id, $dlgTitle)
    }
    if ($dlg -eq [IntPtr]::Zero) { throw "点击按钮后原生文件夹对话框没有弹出" }
    Write-Host "OK: 原生文件夹对话框已弹出"

    $d = [Smoke]::Rect($dlg)
    [Smoke]::SetForegroundWindow($dlg) | Out-Null
    Start-Sleep -Milliseconds 800
    [Smoke]::Click($d[0] + 400, $d[1] + 469)     # 「文件夹:」输入框
    Start-Sleep -Milliseconds 600
    [System.Windows.Forms.SendKeys]::SendWait("^a")
    [System.Windows.Forms.SendKeys]::SendWait($Target)
    Start-Sleep -Milliseconds 800
    [Smoke]::Click($d[0] + 782, $d[1] + 505)     # 「选择文件夹」按钮
    Start-Sleep -Seconds $ResultWait

    if ([Smoke]::Find([uint32]$p.Id, $dlgTitle) -ne [IntPtr]::Zero) {
        throw "对话框没有被关掉，路径可能没被接受"
    }
    Write-Host "OK: 对话框关闭，开始统计"

    [Smoke]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Milliseconds 1200
    $r2 = [Smoke]::Rect($h)
    $w = $r2[2] - $r2[0]; $ht = $r2[3] - $r2[1]
    $bmp = New-Object System.Drawing.Bitmap $w, $ht
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r2[0], $r2[1], 0, 0, (New-Object System.Drawing.Size $w, $ht))
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "OK: 截图已保存 $Out"
}
finally {
    if (-not $p.HasExited) { $p.Kill() }
}
