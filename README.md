# 1vs100-reborn-gameserver

[![build](https://github.com/1vs100/1vs100-reborn-gameserver/actions/workflows/ci.yml/badge.svg)](https://github.com/1vs100/1vs100-reborn-gameserver/actions/workflows/ci.yml)
[![docs](https://img.shields.io/badge/docs-docs.1vs100.fans-blue?logo=readthedocs&logoColor=white)](https://docs.1vs100.fans)
[![download](https://img.shields.io/github/v/release/1vs100/1vs100-reborn-gameserver?include_prereleases&label=download)](https://github.com/1vs100/1vs100-reborn-gameserver/releases)
[![discord](https://img.shields.io/badge/Discord-join-5865F2?logo=discord&logoColor=white)](https://discord.gg/tV3ajhwFTf)
![ai](https://img.shields.io/badge/ai-assisted-orange?logo=micro%3Abit&logoColor=white&label=ai)

tl;dr

- Runs the online services the 1 vs 100 title needs, on your own LAN or hosted on the web.
- Plays a daily programme of Primetime and Extended Play shows.
- Serves the built-in questions or a question bank you import.
- Comes with an admin web UI and an optional public status page.

A self-hosted server for the 1 vs 100 revival on Xbox 360. It provides the online services the game needs, so you can play the show on your own network. Console control (power, pad, HDMI) lives in the separate `x360ccc` repository.

## Getting started

[DEPLOYMENT.md](DEPLOYMENT.md) covers configuration, ports and running the server with Docker Compose, systemd or a shell. The short version with [Docker](https://www.docker.com):

```
cp .env.example .env        # set GAMESERVER_BIND and GAMESERVER_PROTOCOL
docker compose --env-file .env -f deployment/compose.yaml up -d
```

Your host's nginx takes port 80 and forwards it to the stack. Set it up once with [`deployment/host-nginx.conf`](deployment/host-nginx.conf), as DEPLOYMENT.md describes.

Then open the admin UI at `http://127.0.0.1:8080`.

## Daily programme

The server plays Primetime at one or more local times each day and Extended Play in between. Every Primetime show has the same length. The `GAMESERVER_PROGRAMME_*` settings seed the programme on first start. After that you change it on the admin UI's Schedule tab or through the admin API.

An Extended Play show lasts one episode, at most 30 minutes. The retail build v2.0.1149.0 crashes when it joins a show more than 32 minutes after its start.

A special episode replaces one Primetime show, picked by day and start time, and asks only questions of one category. The category must have enough questions at each difficulty for one Primetime round. The Questions tab shows which categories qualify.

## Questions

The server serves 41 built-in questions until you import a question bank. An import takes effect at once and is saved.

`question-import` builds a bank from [OpenTriviaDB](https://opentdb.com) (questions by PIXELTAIL GAMES LLC, CC BY-SA 4.0). Run it on a machine with internet access.

```
cargo run --release -p question-import -- --source otdb --out bank.json
```

It takes about 10 minutes, because the source allows one call every 5 seconds. It prints what it kept and skipped per category, and whether each category is enough for a special. `--categories 9,11` limits it to some of the source's categories.

Every source maps its own categories onto ours (`question-import/src/categories.rs`), so imports from several sources fill the same categories. By default it keeps questions of up to 80 characters and answers of up to 20, ASCII only, because that is what the original show's questions prove fits on screen. `--max-question-chars`, `--max-answer-chars` and `--allow-non-ascii` change that.

Import `bank.json` on the Questions tab or with the admin API.

## Ads

By default the ads port answers the game but offers no ad content. To serve ads, point `GAMESERVER_ADS_CONFIG` at a [massived](https://github.com/1vs100/mads_server_v5) configuration file. It describes the zones and assets. The gameserver supplies the addresses.

The ad client fetches content from the address it is told after its first contact. That is `GAMESERVER_BIND` unless you set `GAMESERVER_PUBLIC_HOST`. Set it when consoles reach the server through NAT or by a DNS name.

## Public site

The public site is a status page you can put on the internet. It shows the show on air, the next show, who is playing and the all-time leaderboard, along with what the project is and how to join. Times appear in each visitor's own time zone.

Players appear on the leaderboard only if they chose Public in the game's privacy settings. The page never shows XUIDs or addresses, and the admin API is never reachable through it. [DEPLOYMENT.md](DEPLOYMENT.md#https) shows how to serve it over HTTPS.

The read-only API behind the page is described as OpenAPI by `cargo run --bin openapi_dump -- status`.

## Admin UI

The admin UI observes the game and edits the programme and the questions. It does not steer a running game.

- **Overview** shows the stored GameState of each program, the matchmaking sessions, log tails and the store. You can view and clear keys there. Each log tail can be filtered by level and text, and lines are coloured by level.
- **Schedule** edits the daily programme with a preview of the next 48 hours before you save. It lists the next 7 or 14 days and manages special episodes. Shows the Grid already lists are locked, so a change keeps them.
- **Questions** shows the categories with their question counts per difficulty. It imports and exports the question bank and lists the served questions with their source, licence and origin.
- **Timeline** shows the server's state timeline by category.
- **Errors** lists every error report the consoles sent, newest first. The server keeps each one, even when a console sends the same error again.
- **SBG** shows the recent SBG frames the consoles sent and got back, decoded. You can filter them by time, connection, console and key. Click a log line's time, or "SBG frames around this report" on an error, to see the frames from that moment. You can also record every frame to a file there and download it for `sbg_replay`.

A tab can be linked, for example `/#schedule` or `/#questions`.

## Admin API

The admin UI uses this API, and you can call it directly. Errors come back as JSON (`{"error": "..."}`) with an HTTP status that says what kind.

```
A=http://127.0.0.1:8081/admin
curl -s $A/health                                           # answers while the server runs
curl -s $A/programme                                        # the daily programme
curl -s -X POST "$A/programme?zone=America/New_York&primetime_at=19:00,22:00&primetime_minutes=60&extended_play_minutes=30"
curl -s $A/shows                                            # shows of the next 48 hours
curl -s "$A/programme/plan?days=7"                          # further ahead
curl -s -X POST "$A/shows/now?mode=pt&minutes=30"           # start a show now (pt or ep), for a test
curl -s -X POST "$A/specials/2026-12-24/20:00?title=Holidays&category=music"
curl -s -X POST $A/specials/2026-12-24/20:00/delete         # that show gets its regular episode back
curl -s -X POST --data-binary @bank.json "$A/questions/import?mode=merge"   # or mode=replace
curl -s $A/questions/export > bank.json
curl -s $A/categories                                       # question counts per difficulty
curl -s "$A/questions?category=music&difficulty=3&limit=20"
curl -s $A/grid/window                                      # what a Grid fetch lists right now
curl -s -X POST $A/reset                                    # drop volatile state, as after a restart
curl -s $A/data/export > dump.json                          # every saved player, document and setting
curl -s -X POST --data-binary @dump.json "$A/data/import?mode=merge"        # or mode=replace
curl -s $A/data/players                                     # players with the sizes of their data
curl -s -X POST $A/data/players/<xuid hex>/delete           # a player and their documents
curl -s -X POST "$A/groups/move?xuid=<xuid hex>&group=2"    # a console into another QNet group
curl -s "$A/errors?limit=50"                                # the consoles' error reports, newest first
curl -s -X POST $A/errors/clear                             # remove every saved error report
```

A question import with `mode=merge` replaces questions by id and adds new ones. `mode=replace` swaps the whole bank.

`reset` clears store values, sums and TopN boards, game-flow bookkeeping, the GameState host and the active session. Player documents, global counters, the programme and matchmaking members stay. Use it between test runs instead of restarting the server.

Starting a show now ends the running one. Consoles hand over to the new show, and the programme continues after it.

## Development

Building, testing and recording sessions are covered in [DEVELOPMENT.md](DEVELOPMENT.md).
