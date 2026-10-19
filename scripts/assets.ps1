$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$assets = Join-Path (Split-Path $PSScriptRoot -Parent) 'installer\assets'
New-Item -ItemType Directory -Path $assets -Force | Out-Null
$navy = [Drawing.Color]::FromArgb(16,29,48)
$mint = [Drawing.Color]::FromArgb(109,218,191)
function Draw-Logo($graphics, $x, $y, $size) {
    $pen = [Drawing.Pen]::new($mint, $size / 18)
    $graphics.DrawEllipse($pen, $x, $y, $size, $size / 3)
    $graphics.DrawArc($pen, $x, $y + $size / 3, $size, $size / 3, 0, 180)
    $graphics.DrawArc($pen, $x, $y + $size * 2 / 3, $size, $size / 3, 0, 180)
    $graphics.DrawLine($pen,$x,$y+$size/6,$x,$y+$size*5/6)
    $graphics.DrawLine($pen,$x+$size,$y+$size/6,$x+$size,$y+$size*5/6)
    $pen.Dispose()
}
$bitmap = [Drawing.Bitmap]::new(410,780)
$g = [Drawing.Graphics]::FromImage($bitmap)
$g.SmoothingMode = 'AntiAlias'
$g.Clear($navy)
Draw-Logo $g 70 75 100
$white = [Drawing.SolidBrush]::new([Drawing.Color]::White)
$accent = [Drawing.SolidBrush]::new($mint)
$font = [Drawing.Font]::new('Segoe UI',23,[Drawing.FontStyle]::Bold)
$small = [Drawing.Font]::new('Segoe UI',14)
$g.DrawString('StrajaBackupSQL',$font,$white,45,235)
$g.DrawString("DATABASE`nBACKUP STUDIO",$small,$accent,49,310)
$g.DrawString("Your data.`nAlways backed up.",$small,$white,49,440)
$g.DrawString("MariaDB  /  MySQL`nPostgreSQL`n`nWindows  /  Rust",$small,$accent,49,600)
$bitmap.Save((Join-Path $assets 'wizard.png'),[Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bitmap.Dispose()
$icon = [Drawing.Bitmap]::new(128,128)
$g = [Drawing.Graphics]::FromImage($icon); $g.SmoothingMode='AntiAlias'; $g.Clear($navy)
Draw-Logo $g 27 22 74
$icon.Save((Join-Path $assets 'logo.png'),[Drawing.Imaging.ImageFormat]::Png)
$rgba = [byte[]]::new(128 * 128 * 4)
for ($y = 0; $y -lt 128; $y++) {
    for ($x = 0; $x -lt 128; $x++) {
        $pixel = $icon.GetPixel($x,$y); $i = ($y * 128 + $x) * 4
        $rgba[$i]=$pixel.R; $rgba[$i+1]=$pixel.G; $rgba[$i+2]=$pixel.B; $rgba[$i+3]=$pixel.A
    }
}
[IO.File]::WriteAllBytes((Join-Path $assets 'logo.rgba'),$rgba)
# PNG-backed ICO; supported on Windows 10 and later.
$png=[IO.File]::ReadAllBytes((Join-Path $assets 'logo.png'))
$stream=[IO.File]::Create((Join-Path $assets 'straja-sql.ico'))
$writer=[IO.BinaryWriter]::new($stream)
$writer.Write([uint16]0);$writer.Write([uint16]1);$writer.Write([uint16]1)
$writer.Write([byte]128);$writer.Write([byte]128);$writer.Write([byte]0);$writer.Write([byte]0)
$writer.Write([uint16]1);$writer.Write([uint16]32);$writer.Write([uint32]$png.Length);$writer.Write([uint32]22);$writer.Write($png)
$writer.Dispose();$g.Dispose();$icon.Dispose();$white.Dispose();$accent.Dispose();$font.Dispose();$small.Dispose()
