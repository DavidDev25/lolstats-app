# lolstats – Tutorial

**Your League companion: what it can do, where to find things and how to read the numbers.**

---

## What is lolstats?

lolstats is a small Windows program that runs alongside League of Legends. It has two parts:

| Part | What it does |
|---|---|
| **Service** (tray icon, bottom right) | Runs invisibly in the background and needs hardly any resources. It watches the League client, records lobbies, results, LP and matches and stores everything in a local database. |
| **Window** | Shows the analyses. Best on a second monitor. Open it with a left click on the tray icon. |

**Where the data comes from:** only your own League client (LCU interface) and the live interface during a game. There is no Riot API key, no server and no upload: everything stays on your PC (`%LOCALAPPDATA%\lolstats`).

**What lolstats never does:** interfere with the game, automate anything in the game or show hidden information (such as enemy cooldowns). Everything is passive – display only.

---

## Getting started

1. **Autostart:** right-click the tray icon → “Start with Windows”. lolstats is then always ready.
2. **Start the League client:** lolstats connects automatically, reads your profile and fetches your recent games. It then completes them in the background, one after another and only outside of games.
3. **Open the window:** left-click the tray icon. F5 or the button at the top right refreshes the display immediately.
4. **Expert mode** (Settings): off shows only the key points, on shows the lab and all tables.
5. **Language** (Settings): German or English, by default like your Windows.

> **Tip:** Leave the client open for a while after playing. Only then does the scraper collect matches of other players (see “The scraper”).

---

## The tabs at a glance

| Tab | What for | When to look |
|---|---|---|
| 📡 **Live** | Lobby, pick suggestions, lane opponent, build, CS tracker, result | in the loading screen, in game, right after |
| 👤 **Me** | Rank, best pick for your rank, break hint, LP history | before your next queue |
| 🛠 **Builds** | Runes, spells, items and boots for every champion | to look things up |
| 🔬 **Lab** | Matchmaking analyses: loser queue test, lobby strength, streaks, team compositions | when you want to understand *why* |
| 📈 **Performance** | Coaching, deaths, farming, playstyle, champion pool, best picks vs … | after a session, to improve |
| 📖 **Tutorial** | this page | when you want to look something up |
| ⚙ **Settings** | Language, expert mode, scraper status, updates, data folder | once |

Everywhere: **hover over a (?) or a value to see an explanation.**

---

## A game with lolstats – from queue to result

### Champ select
- The window can open automatically (Settings).
- **Pick suggestions:** champions you play well against the locked-in enemies, from your own games and all collected matches.
- As soon as you lock in, the window switches to **Live** and shows the **build** for your champion.
- lolstats deliberately does not show names or ranks of your teammates in champ select. Riot hides them there, so we do too. They appear in the loading screen.

### Loading screen
- **Both teams** with rank, form (last 20 games) and experience on the champion.
  - “Offmeta” (gold) means: this champion is very rarely played on this position.
  - Lanes come directly from the client (ranked and draft). In modes without fixed positions lolstats shows no lanes.
- **Lane opponent card:** rank, form, your record against them and the matchup from all matches.
  - **Red warning:** you die before minute 10 more often against this opponent than otherwise. Trade carefully at levels 1–6.
  - **Matchup runes:** runes that players of your champion pick clearly more often against exactly this opponent (from 8 collected games in the matchup). In champ select one click creates the most played page against this opponent in the client.

### In game – CS tracker
Queried every 15 seconds. Riot only reports CS through the live interface in **steps of 10**, for all players, so the value only changes every 1–2 minutes. “Updated X s ago” shows when it was last queried:
- **vs your average:** are you above or below your usual CS at this game time?
- **vs your elo:** comparison with the average of your champion in your elo
- **vs your lane opponent:** CS difference to your direct opponent

Green means ahead, red means behind.

### After the game
- **Result:** victory or defeat, LP and your new rank.
- **Comparison with your recent games:** green is better, red worse than your average. The arrow shows whether the value was higher or lower.
- **Hints:**
  - “±0 LP” after a defeat means loss protection, e.g. because of an AFK teammate. It is detected and noted.
  - **Break hint:** appears after 2 losses in a row or – if your data shows it – after the 3rd game of a session. It is backed by your own numbers, e.g. “from the 4th game you only win 42 %”.

---

## The lab – understanding matchmaking

In expert mode the lab shows what happens around your games:
- **Loser queue test:** do you get worse teammates (trolls, AFK, off-meta, lower ranks) than the enemies after win streaks? With a random test and an honest verdict only from enough games, exportable as CSV.
- **LP vs lobby strength:** each game as a dot, weaker lobbies on the left, stronger ones on the right.
- **Streaks:** win rate and LP after win or losing streaks.
- **Team vs enemy:** win rate when your team is ranked higher or lower on average.
- **Team compositions:** damage profile, crowd control and durability from all collected matches.
- **Log:** every Solo/Duo game with rank before and after and the lobby averages, exportable as CSV.

---

## Performance & coaching – what to work on

**Coaching: what decides your games?** For simple traits your win rate *with* is compared with *without*:
- no death before minute 10
- CS lead at 10 and 15 min
- vision above your average
- at most 5 deaths
- involvement in first blood and the first tower
- gold/min above the elo average of your champion

Green means: with the trait you win at least 10 percentage points more often. Below:
- **Biggest lever:** where you can gain the most
- **Early deaths** per champion and per lane opponent
- **Win rate by game number** in a session
- **CS lead:** do you keep it from minute 10 to 15?
- **Vision vs deaths** in the midgame

**More sections:**
- **Records:** all-time highs and streaks
- **Death timings:** when do you die (early, mid, late)?
- **Farming**
- **Playstyle:** you vs players of your elo on your position
- **Champion pool:** is it too wide for an efficient climb?
- **Best picks vs …:** type an enemy and see fitting picks

> **Important:** these are correlations in *your* games, not proven causes. But they show very well where it pays to look.

---

## The scraper – why more data is better

For builds, matchup runes, elo comparisons and pick suggestions lolstats collects matches of other players:
- **From where:** it starts with the players from your games. Later, players from collected games near your elo are added.
- **When:** only while the League client is open and you are not in champ select or in a game. One query every 2 seconds, no noticeable load.
- **How much:** up to 20,000 matches (Settings). You can see the current status under Settings → Data.
- **More data means more accurate:** the “not enough games” hints disappear over time.

---

## FAQ

**Why does it say “not enough games” everywhere?**
lolstats would rather show nothing than something wrong. With every game the analyses become more reliable: most from 15–20 games, coaching from 10 games per side.

**How do I get updates?**
lolstats checks once a day whether there is a new version and then notifies you in the tray and the sidebar. Under Settings → “Install now” it downloads the new version from GitHub, checks checksum and signature and restarts. A running match is not lost.

**Is this allowed?**
lolstats only reads what your client shows anyway and never interferes with the game. It calculates no rank or elo estimates of its own and does not reveal players in champ select. That is what Riot's policies for third-party tools require.

**Does lolstats cost anything?**
No. lolstats is free and ad-free. If you want to support development: Settings → “Buy me a coffee” or directly at ko-fi.com/daviddev25.

---

## Files

All files are in `%LOCALAPPDATA%\lolstats`. Open the folder via Settings → “Open data folder”.

| File | Content |
|---|---|
| `lolstats.db` | all matches, lobbies, LP snapshots, timelines |
| `log.txt` | log of the service (phases, results, scraper progress) |
| `*.csv` | exports from the lab |
| `config.json` | your settings |

---

lolstats isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or anyone officially involved in producing or managing Riot Games properties. Riot Games, and all associated properties are trademarks or registered trademarks of Riot Games, Inc.
