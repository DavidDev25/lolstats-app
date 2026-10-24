# lolstats – Tutorial

**Dein League-Begleiter: was er kann, wo du was findest und wie du die Zahlen liest.**

---

## Was ist lolstats?

lolstats ist ein kleines Programm für Windows, das neben League of Legends mitläuft. Es besteht aus zwei Teilen:

| Teil | Was er tut |
|---|---|
| **Dienst** (Tray-Icon unten rechts) | Läuft unsichtbar im Hintergrund und braucht kaum Ressourcen. Er beobachtet den League-Client, erfasst Lobbys, Ergebnisse, LP und Matches und speichert alles in einer lokalen Datenbank. |
| **Fenster** | Zeigt die Auswertungen. Am besten auf dem zweiten Monitor. Öffnen per Linksklick auf das Tray-Icon. |

**Woher die Daten kommen:** nur von deinem eigenen League-Client (LCU-Schnittstelle) und der Live-Schnittstelle im Spiel. Es gibt keinen Riot-API-Schlüssel, keinen Server und keinen Upload: Alles bleibt auf deinem PC (`%LOCALAPPDATA%\lolstats`).

**Was lolstats nie tut:** ins Spiel eingreifen, etwas automatisch im Spiel steuern oder versteckte Infos zeigen (etwa die Cooldowns der Gegner). Alles ist passiv, also nur Anzeige.

---

## Erste Schritte

1. **Autostart:** Rechtsklick auf das Tray-Icon → „Mit Windows starten“. lolstats ist dann immer bereit.
2. **League-Client starten:** lolstats verbindet sich automatisch, liest dein Profil und holt deine letzten Spiele. Danach vervollständigt es sie im Hintergrund, eins nach dem anderen und nur außerhalb von Spielen.
3. **Fenster öffnen:** Linksklick aufs Tray-Icon. Mit F5 oder dem Button oben rechts aktualisierst du die Anzeige sofort.
4. **Expertenmodus** (Einstellungen): Aus zeigt nur die Kernaussagen, an zeigt das Labor, die Formeln und alle Tabellen.

> **Tipp:** Lass den Client nach dem Spielen ruhig eine Weile offen. Nur dann sammelt der Scraper Matches anderer Spieler (siehe „Der Scraper“).

---

## Die Tabs im Überblick

<!-- öffentlich -->
| Tab | Wofür | Wann anschauen |
|---|---|---|
| 📡 **Live** | Lobby, Pick-Empfehlung, Lane-Gegner, Build, CS-Tacho, Ergebnis | im Ladebildschirm, im Spiel, direkt danach |
| 👤 **Ich** | Rang, bester Pick für deinen Rang, Pausen-Hinweis, LP-Verlauf | vor der nächsten Queue |
| 🛠 **Builds** | Runen, Zauber, Items und Schuhe für jeden Champion | zum Nachschlagen |
| 🔬 **Labor** | Matchmaking-Auswertungen: Loser-Queue-Test, Lobby-Stärke, Serien, Team-Kompositionen | wenn du verstehen willst, *warum* |
| 📈 **Leistung** | Coaching, Tode, Farmen, Spielstil, Champion-Pool, beste Picks gegen … | nach einer Session, zum Verbessern |
| 📖 **Tutorial** | diese Seite | wenn du etwas nachlesen willst |
| ⚙ **Einstellungen** | Expertenmodus, Scraper-Status, Updates, Datenordner | einmalig |
<!-- /öffentlich -->

Überall gilt: **Fährst du mit der Maus über ein (?) oder einen Wert, erscheint eine Erklärung.**

---

## Ein Spiel mit lolstats – von der Queue bis zum Ergebnis

### Champ Select
- Das Fenster öffnet sich auf Wunsch automatisch (Einstellungen).
- **Pick-Empfehlung:** Champions, die du gegen die gelockten Gegner gut spielst, aus deinen eigenen Spielen und allen gesammelten Matches.
- Sobald du lockst, wechselt das Fenster auf **Live** und zeigt den **Build** für deinen Champion.
<!-- öffentlich -->
- Namen und Ränge deiner Mitspieler zeigt lolstats im Champ Select bewusst nicht an. Riot verbirgt sie dort, also tun wir das auch. Sie erscheinen im Ladebildschirm.
<!-- /öffentlich -->

### Ladebildschirm
- **Beide Teams** mit Rang, Form (letzte 20 Spiele) und Erfahrung auf dem Champion.
  - „Offmeta“ (gold) heißt: Dieser Champion wird auf dieser Position sehr selten gespielt.
<!-- öffentlich -->
  - Lanes kommen direkt vom Client (Ranked und Draft). In Modi ohne feste Positionen zeigt lolstats keine Lanes an.
<!-- /öffentlich -->
- **Lane-Gegner-Karte:** Rang, Form, deine Bilanz gegen ihn und das Matchup aus allen Matches.
  - **Rote Warnung:** Gegen diesen Gegner stirbst du vor Minute 10 öfter als sonst. Level 1–6 also vorsichtig traden.
  - **Matchup-Runen:** Runen, die Spieler deines Champions gegen genau diesen Gegner deutlich häufiger wählen (ab 8 gesammelten Spielen im Matchup).

### Im Spiel – CS-Tacho
Alle 15 Sekunden abgefragt. Riot meldet CS über die Live-Schnittstelle allerdings nur in **10er-Schritten**, für alle Spieler. Der Wert springt deshalb nur etwa alle 1–2 Minuten. „Stand vor X s“ zeigt, wann zuletzt abgefragt wurde:
- **gegen deinen Schnitt:** Liegst du über oder unter deinem üblichen CS zu dieser Spielzeit?
- **gegen die Elo:** Vergleich mit dem Schnitt deines Champions in deiner Elo
- **gegen den Lane-Gegner:** CS-Differenz zu deinem direkten Gegner

Grün heißt vorne, rot heißt hinten.

### Nach dem Spiel
<!-- öffentlich -->
- **Ergebnis:** Sieg oder Niederlage, LP und dein neuer Rang.
<!-- /öffentlich -->
- **Vergleich mit deinen letzten Spielen:** Grün ist besser, rot schlechter als dein Schnitt. Der Pfeil zeigt, ob der Wert höher oder niedriger lag.
- **Hinweise:**
  - „±0 LP“ bei einer Niederlage bedeutet Verlustschutz, zum Beispiel wegen eines AFK-Mitspielers. Er wird erkannt und vermerkt.
  - **Pausen-Hinweis:** Er erscheint nach 2 Niederlagen in Folge oder, wenn deine Daten das belegen, nach dem 3. Spiel einer Session. Er ist mit deinen eigenen Zahlen begründet, zum Beispiel „ab dem 4. Spiel gewinnst du nur 42 %“.

---

<!-- öffentlich -->
## Das Labor – Matchmaking verstehen

Im Expertenmodus zeigt das Labor, was rund um deine Spiele passiert:
- **Loser-Queue-Test:** Bekommst du nach Siegesserien schlechtere Mitspieler (Trolls, AFK, Offmeta, niedrigere Ränge) als die Gegner? Mit Zufallstest und ehrlichem Urteil erst ab genug Spielen, als CSV exportierbar.
- **LP gegen Lobby-Stärke:** jedes Spiel als Punkt, links schwächere, rechts stärkere Lobbys als dein Rang.
- **Serien:** Siegchance und LP nach Sieg- bzw. Niederlagenserien.
- **Team gegen Gegner:** Siegchance, wenn dein Team im Schnitt höher oder niedriger eingestuft ist.
- **Team-Kompositionen:** Schadensprofil, Crowd Control und Haltbarkeit aus allen gesammelten Matches.
- **Protokoll:** jedes Solo/Duo-Spiel mit Rang vorher und nachher und den Lobby-Schnitten, als CSV exportierbar.

---
<!-- /öffentlich -->

## Leistung & Coaching – woran du arbeiten kannst

**Coaching: Was entscheidet deine Spiele?** Für einfache Merkmale wird deine Siegchance *mit* gegen *ohne* verglichen:
- kein Tod vor Minute 10
- CS-Vorsprung bei 10 und 15 min
- Vision über deinem Schnitt
- höchstens 5 Tode
- Beteiligung an First Blood und am ersten Turm
- Gold/min über dem Elo-Schnitt deines Champions

Grün heißt: Mit dem Merkmal gewinnst du mindestens 10 Prozentpunkte öfter. Darunter stehen:
- **Größter Hebel:** wo du am meisten gewinnen kannst
- **Frühe Tode** je Champion und je Lane-Gegner
- **Siegchance nach Spielnummer** in der Session
- **CS-Vorsprung:** hältst du ihn von Minute 10 bis 15?
- **Vision gegen Tode** im Midgame

**Weitere Bereiche:**
- **Rekorde:** Allzeit-Hochs und Serien
- **Death-Timings:** Wann stirbst du (Early, Mid, Late)?
- **Farmen**
- **Spielstil:** du gegen Spieler deiner Elo auf deiner Position
- **Champion-Pool:** Ist er zu breit für einen effizienten Climb?
- **Beste Picks gegen …:** einen Gegner eintippen und passende Picks sehen

> **Wichtig:** Das alles sind Zusammenhänge in *deinen* Spielen, keine bewiesenen Ursachen. Sie zeigen aber sehr gut, wo es sich lohnt hinzuschauen.

---

## Der Scraper – warum mehr Daten besser sind

Für Builds, Matchup-Runen, Elo-Vergleiche und Pick-Empfehlungen sammelt lolstats Matches anderer Spieler:
- **Woher:** Er startet bei den Mitspielern aus deinen Spielen. Später kommen auch Spieler aus gesammelten Spielen nahe deiner Elo dazu.
- **Wann:** nur, wenn der League-Client offen ist und du nicht in Champ Select oder im Spiel bist. Er macht eine Abfrage alle 2 Sekunden und belastet nichts.
- **Wie viel:** bis zu 20 000 Matches (Einstellungen). Den aktuellen Stand siehst du unter Einstellungen → Daten.
- **Mehr Daten heißt genauer:** Die Hinweise „zu wenige Spiele“ verschwinden nach und nach.

---

## Häufige Fragen

**Warum steht überall „zu wenige Spiele“?**
lolstats zeigt lieber nichts als etwas Falsches. Mit jedem Spiel werden die Aussagen belastbarer: die meisten Auswertungen ab 15–20 Spielen, Coaching ab 10 Spielen je Seite.

<!-- öffentlich -->
**Wie bekomme ich Updates?**
lolstats sieht einmal am Tag nach, ob es eine neue Version gibt, und meldet sich dann im Tray und in der Seitenleiste. Unter Einstellungen → „Jetzt installieren“ lädt es die neue Version von GitHub, prüft die Prüfsumme und startet neu. Ein laufendes Match geht dabei nicht verloren.

**Ist das erlaubt?**
lolstats liest nur, was dein Client ohnehin anzeigt, und greift nie ins Spiel ein. Es berechnet keine eigenen Rang- oder Elo-Schätzungen und deckt im Champ Select keine Spieler auf. So verlangen es die Riot-Richtlinien für Drittanbieter-Tools.

**Kostet lolstats etwas?**
Nein. lolstats ist kostenlos und werbefrei. Wenn du die Entwicklung unterstützen möchtest: Einstellungen → „Einen Kaffee spendieren“ oder direkt auf ko-fi.com/daviddev25.
<!-- /öffentlich -->

---

## Dateien

Alle Dateien liegen in `%LOCALAPPDATA%\lolstats`. Den Ordner öffnest du über Einstellungen → „Datenordner öffnen“.

| Datei | Inhalt |
|---|---|
| `lolstats.db` | alle Matches, Lobbys, LP-Stände, Timelines |
| `log.txt` | Protokoll des Dienstes (Phasen, Ergebnisse, Scraper-Fortschritt) |
| `*.csv` | Exporte aus dem Labor |
| `config.json` | deine Einstellungen |

---

lolstats isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or anyone officially involved in producing or managing Riot Games properties. Riot Games, and all associated properties are trademarks or registered trademarks of Riot Games, Inc.
