#!/usr/bin/env bash
# Run the stack's nginx behind the host's nginx: the public site, the status API and the
# consoles' Grid requests on GAMESERVER_SITE_LISTEN, and the admin UI on GAMESERVER_UI_LISTEN.
#
#   deployment/nginx/serve.sh run|start|stop|restart|reload|status|test
#
# `run` stays in the foreground (systemd, containers); the others daemonize.
# Settings come from the environment (see .env.example).
# Optional: ONEVS100_ROOT (the directory holding frontend/dist and site/dist, default the
# repository), NGINX_RUN_DIR (deployment/nginx/run), NGINX_MIME_TYPES, NGINX_USER (the user
# the workers run as when started as root, default nginx's built-in one), NGINX_REAL_IP_FROM
# (addresses or ranges besides loopback the host's nginx connects from, separated by spaces).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${ONEVS100_ROOT:-$(dirname "$(dirname "$here")")}"
grid="${GAMESERVER_HTTP_ADDR:?GAMESERVER_HTTP_ADDR must name the gameserver HTTP address nginx forwards to (for example 127.0.0.1:8090)}"
listen="${GAMESERVER_SITE_LISTEN:-127.0.0.1:8083}"
status="${GAMESERVER_STATUS_ADDR:-127.0.0.1:8082}"
ui_listen="${GAMESERVER_UI_LISTEN:-127.0.0.1:8080}"
admin="${GAMESERVER_ADMIN_ADDR:-127.0.0.1:8081}"
run="${NGINX_RUN_DIR:-$here/run}"
mime="${NGINX_MIME_TYPES:-/etc/nginx/mime.types}"
user="${NGINX_USER:-}"
real_ip_from="127.0.0.0/8 ::1 ${NGINX_REAL_IP_FROM:-}"
conf="$run/nginx.conf"

# ip:port, IPv4 or bracketed IPv6; anything else would break the rendered configuration.
address() {
  [[ "$2" =~ ^([0-9.]+|\[[0-9a-fA-F:]+\]):[0-9]+$ ]] || { echo "$1 must be ip:port, got '$2'" >&2; exit 1; }
}

# ip:port or host:port: a gameserver nginx forwards to, by name on a compose network.
upstream() {
  [[ "$2" =~ ^([0-9.]+|\[[0-9a-fA-F:]+\]|[A-Za-z0-9.-]+):[0-9]+$ ]] || { echo "$1 must be ip:port or host:port, got '$2'" >&2; exit 1; }
}

fill() {
  sed -e "s|__RUN__|$run|g" -e "s|__LISTEN__|$listen|g" \
      -e "s|__GRID__|$grid|g" -e "s|__STATUS__|$status|g" \
      -e "s|__UI_LISTEN__|$ui_listen|g" -e "s|__ADMIN__|$admin|g" -e "s|__ROOT__|$root|g" \
      -e "s|__RESOLVER__|$resolver|g" -e "s|__REAL_IP_FROM__|$trusted|g" \
      -e "s|__MIME__|$mime|g" "$here/$1"
}

render() {
  address GAMESERVER_SITE_LISTEN "$listen"
  upstream GAMESERVER_HTTP_ADDR "$grid"
  upstream GAMESERVER_STATUS_ADDR "$status"
  address GAMESERVER_UI_LISTEN "$ui_listen"
  upstream GAMESERVER_ADMIN_ADDR "$admin"
  [[ -z "$user" || "$user" =~ ^[A-Za-z0-9._-]+$ ]] || { echo "NGINX_USER must be a user name, got '$user'" >&2; exit 1; }
  trusted=""
  local from
  for from in $real_ip_from; do
    [[ "$from" =~ ^[0-9a-fA-F.:]+(/[0-9]+)?$ ]] || { echo "NGINX_REAL_IP_FROM must list addresses or ranges, got '$from'" >&2; exit 1; }
    trusted+="set_real_ip_from $from; "
  done
  # A named upstream is looked up through the DNS server the host or container uses.
  resolver=""
  local upstream_host
  for upstream_host in "${grid%:*}" "${status%:*}" "${admin%:*}"; do
    [[ "$upstream_host" =~ ^([0-9.]+|\[[0-9a-fA-F:]+\])$ ]] && continue
    local server
    server=$(awk '$1 == "nameserver" { print $2; exit }' /etc/resolv.conf)
    [[ -n "$server" ]] || { echo "no nameserver in /etc/resolv.conf to look up $upstream_host" >&2; exit 1; }
    [[ "$server" != *:* ]] || server="[$server]"
    resolver="resolver $server valid=10s;"
    break
  done
  [[ "$grid" != "$listen" ]] || { echo "GAMESERVER_HTTP_ADDR ($grid) is where nginx listens; move the gameserver HTTP elsewhere" >&2; exit 1; }
  for dist in "$root/site/dist" "$root/frontend/dist"; do
    [[ -f "$dist/index.html" ]] || { echo "no build at $dist - run 'pnpm build' first" >&2; exit 1; }
  done
  [[ -f "$mime" ]] || { echo "mime types not found at $mime (set NGINX_MIME_TYPES)" >&2; exit 1; }
  mkdir -p "$run/tmp"
  { [[ -z "$user" ]] || echo "user $user;"; fill nginx.conf.in; } > "$conf"
}

pid() { [[ -f "$run/nginx.pid" ]] && kill -0 "$(cat "$run/nginx.pid")" 2>/dev/null && cat "$run/nginx.pid"; }
nx() { nginx -p "$run" -e "$run/error.log" -c "$conf" "$@"; }

case "${1:-}" in
  run)     render; exec nginx -p "$run" -e stderr -c "$conf" -g 'daemon off; error_log stderr warn;' ;;
  test)    render; nx -t ;;
  start)   if p=$(pid); then echo "already running (pid $p)"; else render; nx -t; nx -g "error_log $run/error.log warn;"; echo "serving http://$listen/ and the admin UI on http://$ui_listen/"; fi ;;
  stop)    if p=$(pid); then nx -s quit; echo "stopped (pid $p)"; else echo "not running"; fi ;;
  reload)  render; nx -t; nx -s reload; echo "reloaded" ;;
  restart) "$0" stop; for _ in $(seq 1 50); do pid >/dev/null || break; sleep 0.1; done; "$0" start ;;
  status)  if p=$(pid); then echo "running (pid $p) on $listen and $ui_listen"; else echo "not running"; exit 1; fi ;;
  *)       echo "usage: $0 run|start|stop|restart|reload|status|test" >&2; exit 2 ;;
esac
