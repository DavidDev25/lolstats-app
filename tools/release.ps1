# Baut lolstats und legt sie releasefertig auf den Desktop:
#   Desktop\lolstats-public\v<Version>\lolstats.exe          (zum Hochladen)
#   Desktop\lolstats-public\v<Version>\lolstats.exe.sha256   (Pruefsumme fuer den Auto-Updater)
#   Desktop\lolstats-public\v<Version>\LIESMICH.txt
#   Desktop\lolstats-public\v<Version>\RELEASE-NOTES.md      (Text fuer das GitHub-Release)
# Aufruf: powershell -ExecutionPolicy Bypass -File tools\release.ps1
# Eigener Build-Ordner (target\public) mit neutralisierten Quellpfaden.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$env:LOCALAPPDATA\Programs\w64devkit\bin;$env:PATH"

# Datenschutz: Quellpfade in der exe (Panic-Meldungen) enthalten sonst den Windows-Benutzernamen
# (C:\Users\<Name>\.cargo\...) und den Projektordner -> durch neutrale Namen ersetzen.
# RUSTFLAGS ersetzt die rustflags aus .cargo\config.toml, deshalb die Linker-Flags hier mitgeben.
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
$shim = (Join-Path $root ".cargo\gnu-shim") -replace '\\', '/'
$env:RUSTFLAGS = "-L native=$shim -Cdlltool=dlltool --remap-path-prefix=$cargoHome=cargo --remap-path-prefix=$root=lolstats --remap-path-prefix=$env:USERPROFILE=~"

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "lolstats $version"

Write-Host "Tests ..."
cargo test --release --target-dir target\public
if ($LASTEXITCODE -ne 0) { throw "Tests fehlgeschlagen - kein Release." }

Write-Host "Build ..."
cargo build --release --target-dir target\public
if ($LASTEXITCODE -ne 0) { throw "Build fehlgeschlagen." }

$out = Join-Path ([Environment]::GetFolderPath("Desktop")) "lolstats-public\v$version"
New-Item -ItemType Directory -Force $out | Out-Null
$exe = Join-Path $out "lolstats.exe"
Copy-Item target\public\release\lolstats.exe $exe -Force

# Kontrolle: keine persoenlichen Pfade in der exe
$text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($exe))
foreach ($leak in @($env:USERPROFILE, $root)) {
    if ($text.Contains($leak)) { throw "Die exe enthaelt noch den Pfad '$leak' - Release abgebrochen." }
}

$hash = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
$utf8 = New-Object System.Text.UTF8Encoding($false)
[IO.File]::WriteAllText((Join-Path $out "lolstats.exe.sha256"), "$hash  lolstats.exe`n", $utf8)

$readme = @"
lolstats $version - kostenloser League-of-Legends-Begleiter fuer Windows

Installation
  1. lolstats.exe in einen eigenen Ordner legen (z. B. Dokumente\lolstats) und starten.
     Windows zeigt beim ersten Start evtl. "Der Computer wurde durch Windows geschuetzt":
     "Weitere Informationen" -> "Trotzdem ausfuehren" (die exe ist nicht kostenpflichtig signiert).
  2. Unten rechts erscheint das lolstats-Symbol. Linksklick oeffnet das Fenster,
     Rechtsklick -> "Mit Windows starten" fuer den Autostart.
  3. League-Client starten - lolstats verbindet sich automatisch.
  Im Fenster unter "Tutorial" steht alles Weitere.

Updates
  lolstats prueft einmal am Tag auf github.com/DavidDev25/lolstats-app und installiert
  neue Versionen auf Knopfdruck (Einstellungen). Jede Datei wird per SHA-256 geprueft.

Pruefsumme dieser Datei (SHA-256)
  $hash

Datenschutz
  Alle Daten bleiben auf deinem PC (%LOCALAPPDATA%\lolstats). Kein Konto, kein Upload, kein Riot-API-Schluessel.

Unterstuetzen
  lolstats ist kostenlos und werbefrei. Wer die Entwicklung unterstuetzen moechte: https://ko-fi.com/daviddev25

lolstats isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games
or anyone officially involved in producing or managing Riot Games properties. Riot Games, and all
associated properties are trademarks or registered trademarks of Riot Games, Inc.
"@
[IO.File]::WriteAllText((Join-Path $out "LIESMICH.txt"), $readme, $utf8)

$notes = Join-Path $out "RELEASE-NOTES.md"
if (-not (Test-Path $notes)) {
    $template = @"
## lolstats $version

### Neu
- ...

### Verbessert
- ...

SHA-256 (lolstats.exe): ``$hash``
"@
    [IO.File]::WriteAllText($notes, $template, $utf8)
}

$size = [math]::Round((Get-Item $exe).Length / 1MB, 1)
Write-Host ""
Write-Host "Fertig: $out"
Write-Host "  lolstats.exe ($size MB), SHA-256 $hash"
Write-Host ""
Write-Host "Veroeffentlichen (GitHub, Repo DavidDev25/lolstats-app):"
Write-Host "  1. Neues Release mit Tag v$version anlegen, Text aus RELEASE-NOTES.md"
Write-Host "  2. lolstats.exe und lolstats.exe.sha256 als Dateien anhaengen"
Write-Host "  Der Auto-Updater findet das Release ueber 'releases/latest'."
