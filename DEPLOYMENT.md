# Deployment

tl;dr

- Runs the gameserver and its own [nginx](https://nginx.org) with Docker Compose, systemd or a shell.
- Your host's nginx owns port 80 and HTTPS on `GAMESERVER_BIND` and forwards to the stack.
- The stack serves the public site, the consoles' requests and the admin UI, all on loopback.

Everything described here lives in [`deployment/`](deployment). Features and the admin API are covered in [README.md](README.md).

## How it fits together

- The consoles reach the game ports (SBG, matchmaking, ads and time) directly on `GAMESERVER_BIND`.
- The consoles and visitors send HTTP to port 80 on `GAMESERVER_BIND`. Your host's nginx takes it there and forwards it to the stack's nginx on `127.0.0.1:8083`.
- The stack's nginx serves the public site and passes the consoles' requests and the status API on to the gameserver.
- The admin UI is served by the stack's nginx on `127.0.0.1:8080`. It does not go through the host's nginx.

The host's nginx only routes. Everything else comes with the stack, so updating the stack never touches your host's configuration.

## Configure

All settings are environment variables, documented in [`.env.example`](.env.example). Copy it and adjust it. The copy is git-ignored.

```
cp .env.example .env
```

You must set two values.

- `GAMESERVER_BIND` is the IP the consoles reach this host on. It must be a concrete address, not `0.0.0.0`, because the console is told to connect there.
- `GAMESERVER_PROTOCOL` is the title build this instance serves, `1149` or `1317`. One instance serves one build. To serve both, run two instances on two addresses.

The admin API and the admin UI have no authentication. Keep `GAMESERVER_ADMIN_ADDR` and `GAMESERVER_UI_LISTEN` on loopback or a trusted network.

## Ports

Open these on `GAMESERVER_BIND` for the consoles.

| Service | 1149 | 1317 |
|---|---|---|
| HTTP (Grid, livehost, public site), on the host's nginx | tcp 80 | tcp 80 |
| SBG | tcp 1000 | tcp 1000 |
| Matchmaking | tcp 1001 | tcp 1001 |
| Ads | tcp 1002 | tcp 1002 |
| Time (SNTP) | udp 1123 | udp 123 |

Port 443 serves the public site once HTTPS is set up.

The admin UI listens on `127.0.0.1:8080`. To reach it from another machine, use an SSH tunnel such as `ssh -L 8080:127.0.0.1:8080 <host>`.

## Set up the host's nginx

Every setup below needs this once. Install nginx 1.22 or later from your distribution, if it is not there yet.

```
apt install nginx
```

Copy the sample configuration and put your `GAMESERVER_BIND` in place of `192.0.2.10`. Then check it and reload nginx.

```
cp deployment/host-nginx.conf /etc/nginx/conf.d/onevs100.conf
sed -i 's/192\.0\.2\.10/<GAMESERVER_BIND>/' /etc/nginx/conf.d/onevs100.conf
nginx -t && systemctl reload nginx
```

Your nginx can keep serving other sites. Give `GAMESERVER_BIND` to the gameserver alone if you can. nginx sends every request to an address named in a `listen` to the blocks that name it, so a site with a plain `listen 80;` does not get requests on `GAMESERVER_BIND`. If your other sites share that address, add `listen <GAMESERVER_BIND>:80;` to each of them.

The sample does four things. Keep them if you change it or use another web server.

- It listens on `GAMESERVER_BIND:80` and answers every request there, whatever host name it carries. The consoles' requests land in it.
- It forwards everything to the stack's nginx on `127.0.0.1:8083`.
- It passes the `Host` header unchanged with `$http_host`. The Grid signs its replies over it, and the console rejects a reply signed for another host. `$host` does not work, because it drops the port and changes the case.
- It sends the visitor's address in `X-Real-IP`. The stack's nginx limits each visitor by it.

## Run with Docker Compose

This runs the images built from the latest `main`. You only need the `deployment/` directory, with your `.env` next to it.

Run every command as root, from the directory that holds `deployment/` and `.env`.

```
docker compose --env-file .env -f deployment/compose.yaml up -d
```

Every compose command needs `--env-file .env`. Compose reads `GAMESERVER_BIND` from it to know which address to publish the game ports on.

If the images are private, run `docker login ghcr.io` first. [Podman](https://podman.io) works the same with `podman compose`.

Compose publishes these ports.

- The game ports from the table above, on `GAMESERVER_BIND`. Both time ports are published, so either build works.
- The stack's nginx on `127.0.0.1:8083`, for the host's nginx.
- The admin UI on `127.0.0.1:8080`.

The admin and status APIs are not published. The stack's nginx reaches them inside compose.

Compose also runs a [PostgreSQL](https://www.postgresql.org) database for the gameserver's data. It sits on a network of its own that only the gameserver joins, and it publishes no port. The database and the logs live in named volumes.

To open a SQL shell on it, run this.

```
docker compose --env-file .env -f deployment/compose.yaml exec postgres psql -U onevs100
```

Compose sets the gameserver's own addresses inside the container. It ignores `GAMESERVER_HTTP_ADDR`, `GAMESERVER_ADMIN_ADDR` and `GAMESERVER_STATUS_ADDR` in `.env`.

Compose must run as root because the gameserver tells consoles apart by their address. Rootless Docker and rootless Podman make every console arrive from the same address.

To serve ads, start the stack once, then put the massived configuration and its content into the `ads` volume, which the gameserver mounts read-only at `/ads`. Then set `GAMESERVER_ADS_CONFIG=/ads/massived.toml` in `.env` and recreate the gameserver. The volume is read-only, so leave the `[capture]` section out of the configuration or point its path at `/logs`.

```
docker run --rm -v onevs100_ads:/ads -v "$PWD/ads:/src:ro" alpine cp -r /src/. /ads/
```

To follow the logs, run this.

```
docker compose --env-file .env -f deployment/compose.yaml logs -f
```

To update, pull the new images and recreate the containers.

```
docker compose --env-file .env -f deployment/compose.yaml pull
docker compose --env-file .env -f deployment/compose.yaml up -d
```

### Move data from an earlier SQLite install

Earlier versions of the stack kept their data in SQLite, in the `onevs100_data` volume. An update starts on an empty PostgreSQL database. To keep your players, programme and questions, export them before you update and import them after.

Export first, and check that the file is a dump. `jq` prints its version.

```
curl -fsS http://127.0.0.1:8080/admin/data/export > dump.json
jq .version dump.json
```

Then update, wait until the admin API answers, and import the dump. The import prints what it loaded.

```
docker compose --env-file .env -f deployment/compose.yaml pull
docker compose --env-file .env -f deployment/compose.yaml up -d
until curl -fsS http://127.0.0.1:8080/admin/health > /dev/null; do sleep 2; done
curl -fsS -X POST --data-binary @dump.json "http://127.0.0.1:8080/admin/data/import?mode=replace"
```

Check the admin UI before you delete anything. The old volume stays until you remove it with `docker volume rm onevs100_data`.

### Build from source

To run your own checkout, add `compose.build.yaml`. It builds both images locally instead of pulling them.

```
docker compose --env-file .env -f deployment/compose.yaml -f deployment/compose.build.yaml up -d --build
```

To update, pull the new code and run the same command again.

## Run with systemd

Build the server and both web pages first. You need [Rust](https://rustup.rs) and [pnpm](https://pnpm.io).

```
cargo build --release
(cd frontend && pnpm install && pnpm build)
(cd site && pnpm install && pnpm build)
```

Then install them under `/opt/onevs100`. The copies replace what an earlier install left there, so no old build files stay behind.

```
install -D target/release/onevs100-gameserver /opt/onevs100/onevs100-gameserver
rm -rf /opt/onevs100/frontend/dist /opt/onevs100/site/dist /opt/onevs100/deployment
mkdir -p /opt/onevs100/frontend /opt/onevs100/site
cp -r frontend/dist /opt/onevs100/frontend/
cp -r site/dist /opt/onevs100/site/
cp -r deployment /opt/onevs100/
install -D .env /etc/onevs100/gameserver.env
cp deployment/systemd/*.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now onevs100-gameserver onevs100-nginx
```

`onevs100-nginx` is the stack's nginx. It runs as its own unprivileged instance next to the host's nginx service.

Data and logs live in `/var/lib/onevs100`. The data is a SQLite file there. To use a PostgreSQL database instead, set `GAMESERVER_DATABASE_URL` in `/etc/onevs100/gameserver.env`, such as `postgres://onevs100:<password>@127.0.0.1/onevs100`. The server creates its tables on first start, so its user must own the database.

```
CREATE USER onevs100 PASSWORD '<password>';
CREATE DATABASE onevs100 OWNER onevs100;
```

The server connects without TLS, so keep the database on the same host or a trusted network.

To follow the gameserver's output, run `journalctl -fu onevs100-gameserver -o cat`.

To update, run `systemctl stop onevs100-nginx` first, because the copy steps replace the files it runs from. Then repeat the build and the copy steps, and run `systemctl restart onevs100-gameserver onevs100-nginx`.

## Run from a shell

This is handy for testing on your own machine. Ports 123 and 1000 need root or `CAP_NET_BIND_SERVICE`.

```
cargo build --release
set -a; . ./.env; set +a
./target/release/onevs100-gameserver
```

In a second shell, after building both web pages as above, start the stack's nginx.

```
set -a; . ./.env; set +a
deployment/nginx/serve.sh start
```

`serve.sh` also takes `stop`, `restart`, `reload`, `status` and `test`.

## HTTPS

The public site can use a [Let's Encrypt](https://letsencrypt.org) certificate on the host's nginx. The consoles keep using plain HTTP on port 80.

Point a DNS name at the host. Put it in `server_name` in `/etc/nginx/conf.d/onevs100.conf`, in place of `_`.

```
server_name status.example.org;
```

Then let [certbot](https://certbot.eff.org) add HTTPS to that server block.

```
certbot --nginx --no-redirect -d status.example.org
```

`--no-redirect` keeps port 80 answering over HTTP. The consoles do not speak HTTPS, so a redirect would break them if they reach the server by the same name. certbot's timer renews the certificate and reloads nginx.

certbot adds `listen 443 ssl` without an address. If nginx serves other HTTPS sites, mark one of them `default_server`, so a request for an unknown name does not land on the 1 vs 100 site.

## Check that it runs

- `curl -s http://127.0.0.1:8080/admin/health` answers. Use your `GAMESERVER_UI_LISTEN` if you changed it.
- `curl -s http://<GAMESERVER_BIND>/status/v1/server` answers through the host's nginx.
- The log shows `all sockets bound` and `admin API listening`.
- `ss -tulpn` lists the game ports above on `GAMESERVER_BIND`.

All ports must bind before any service starts. If one is taken, the gameserver exits and names it.

Consoles see a restart as a backend outage. Do not restart during a live game.
