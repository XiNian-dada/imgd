#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<USAGE
Usage:
  sudo ./deploy/install.sh [options]

Interactive mode:
  If run in a TTY, the script will ask for key settings.
  Press Enter to accept defaults.

Options:
  --domain <domain>            Public domain, e.g. img.example.com
  --token <upload_token>       Optional legacy UPLOAD_TOKEN (not recommended)
  --bin <path>                 Binary path (default: ./imgd)
  --port <port>                Backend listen port (default: random free 4-digit)
  --data-dir <dir>             Image data dir (default: /data/images)
  --public-base-url <url>      Public image base URL (default: https://<domain>/images)
  --service-user <user>        Service user (default: imgd)
  --rate-limit <n>             RATE_LIMIT_PER_MINUTE (default: 60)
  --max-concurrent <n>         MAX_CONCURRENT_UPLOADS (default: 16)
  --skip-nginx                 Skip nginx install/config
  --no-enable                  Do not enable/start services
  --non-interactive            Disable prompts

Example:
  sudo ./deploy/install.sh \
    --domain img.example.com \
    --bin ./imgd
USAGE
}

DOMAIN=""
UPLOAD_TOKEN=""
BIN_PATH="./imgd"
PORT=""
DATA_DIR="/data/images"
PUBLIC_BASE_URL=""
SERVICE_USER="imgd"
RATE_LIMIT_PER_MINUTE="60"
MAX_CONCURRENT_UPLOADS="16"
SKIP_NGINX="0"
NO_ENABLE="0"
INTERACTIVE="auto"
EXISTING_ENV_FILE="/opt/imgd/conf/imgd.env"

if [[ -t 1 ]]; then
  C_BOLD="$(printf '\033[1m')"
  C_BLUE="$(printf '\033[34m')"
  C_YELLOW="$(printf '\033[33m')"
  C_RED="$(printf '\033[31m')"
  C_RESET="$(printf '\033[0m')"
else
  C_BOLD=""
  C_BLUE=""
  C_YELLOW=""
  C_RED=""
  C_RESET=""
fi

step() { echo "${C_BOLD}${C_BLUE}==>${C_RESET} $*"; }
warn() { echo "${C_YELLOW}WARN:${C_RESET} $*"; }
fail() { echo "${C_RED}ERROR:${C_RESET} $*" >&2; exit 1; }

pick_random_port() {
  local p
  for _ in $(seq 1 200); do
    p=$((RANDOM % 9000 + 1000))
    if port_is_free "$p"; then
      echo "$p"
      return 0
    fi
  done
  echo "3000"
}

port_is_free() {
  local p="$1"
  if command -v lsof >/dev/null 2>&1; then
    ! lsof -iTCP:"$p" -sTCP:LISTEN -Pn >/dev/null 2>&1
    return
  fi
  if command -v ss >/dev/null 2>&1; then
    ! ss -ltn "sport = :$p" 2>/dev/null | tail -n +2 | grep -q .
    return
  fi
  if command -v netstat >/dev/null 2>&1; then
    ! netstat -lnt 2>/dev/null | awk '{print $4}' | grep -E "[:.]$p$" >/dev/null 2>&1
    return
  fi
  return 0
}

prompt_with_default() {
  local prompt="$1"
  local default="$2"
  local value
  read -r -p "$prompt [$default]: " value
  if [[ -z "$value" ]]; then
    echo "$default"
  else
    echo "$value"
  fi
}

discover_nginx_conf() {
  local v conf_path prefix
  v="$(nginx -V 2>&1 || true)"
  conf_path="$(echo "$v" | sed -n 's/.*--conf-path=\([^ ]*\).*/\1/p')"
  prefix="$(echo "$v" | sed -n 's/.*--prefix=\([^ ]*\).*/\1/p')"

  if [[ -z "$conf_path" && -n "$prefix" ]]; then
    conf_path="$prefix/conf/nginx.conf"
  fi
  if [[ -z "$conf_path" && -f /etc/nginx/nginx.conf ]]; then
    conf_path="/etc/nginx/nginx.conf"
  fi

  [[ -n "$conf_path" ]] || return 1
  [[ -f "$conf_path" ]] || return 1

  NGINX_CONF_PATH="$conf_path"
  NGINX_PREFIX="$prefix"
  export NGINX_CONF_PATH NGINX_PREFIX
  return 0
}

resolve_nginx_path() {
  local p="$1"
  local base
  if [[ "$p" = /* ]]; then
    echo "$p"
    return 0
  fi
  if [[ -n "${NGINX_PREFIX:-}" ]]; then
    base="$NGINX_PREFIX"
  else
    base="$(dirname "${NGINX_CONF_PATH}")"
  fi
  echo "${base%/}/$p"
}

pick_nginx_include_dir() {
  local include raw dir best http_includes
  best=""
  http_includes="$(awk '
    BEGIN { in_http=0; depth=0 }
    function count_char(str, ch,   i, n, c) {
      n=0
      for (i=1; i<=length(str); i++) if (substr(str,i,1)==ch) n++
      return n
    }
    {
      line=$0
      if (!in_http && line ~ /^[[:space:]]*http[[:space:]]*\{/) {
        in_http=1
        depth=1
        next
      }
      if (in_http) {
        if (line ~ /^[[:space:]]*include[[:space:]]+[^;]+;/) print line
        depth += count_char(line, "{")
        depth -= count_char(line, "}")
        if (depth <= 0) in_http=0
      }
    }
  ' "$NGINX_CONF_PATH")"

  if [[ -z "$http_includes" ]]; then
    http_includes="$(grep -E '^[[:space:]]*include[[:space:]]+[^;]+;' "$NGINX_CONF_PATH" || true)"
  fi

  while IFS= read -r raw; do
    [[ -z "$raw" ]] && continue
    include="$(echo "$raw" | sed -E 's/^[[:space:]]*include[[:space:]]+([^;]+);[[:space:]]*$/\1/')"
    include="$(resolve_nginx_path "$include")"
    if [[ "$include" == *"*"* ]]; then
      dir="${include%/*}"
    else
      dir="$(dirname "$include")"
    fi
    [[ -d "$dir" ]] || continue
    case "$dir" in
      */tcp*|*/stream*)
        continue
        ;;
    esac
    case "$dir" in
      *vhost*|*sites-enabled*|*conf.d*)
        echo "$dir"
        return 0
        ;;
      *)
        [[ -z "$best" ]] && best="$dir"
        ;;
    esac
  done <<< "$http_includes"

  [[ -n "$best" ]] && echo "$best" && return 0
  return 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --domain)
      DOMAIN="$2"; shift 2 ;;
    --token)
      UPLOAD_TOKEN="$2"; shift 2 ;;
    --bin)
      BIN_PATH="$2"; shift 2 ;;
    --port)
      PORT="$2"; shift 2 ;;
    --data-dir)
      DATA_DIR="$2"; shift 2 ;;
    --public-base-url)
      PUBLIC_BASE_URL="$2"; shift 2 ;;
    --service-user)
      SERVICE_USER="$2"; shift 2 ;;
    --rate-limit)
      RATE_LIMIT_PER_MINUTE="$2"; shift 2 ;;
    --max-concurrent)
      MAX_CONCURRENT_UPLOADS="$2"; shift 2 ;;
    --skip-nginx)
      SKIP_NGINX="1"; shift ;;
    --no-enable)
      NO_ENABLE="1"; shift ;;
    --non-interactive)
      INTERACTIVE="0"; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 1 ;;
  esac
done

if [[ "$INTERACTIVE" == "auto" ]]; then
  if [[ -t 0 && -t 1 ]]; then
    INTERACTIVE="1"
  else
    INTERACTIVE="0"
  fi
fi

SUGGESTED_PORT="$(pick_random_port)"
DEFAULT_DOMAIN="$(hostname -f 2>/dev/null || hostname || echo img.local)"

if [[ -f "$EXISTING_ENV_FILE" ]]; then
  existing_port="$(grep -E '^PORT=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"
  existing_data_dir="$(grep -E '^DATA_DIR=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"
  existing_public_base_url="$(grep -E '^PUBLIC_BASE_URL=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"
  existing_max_concurrent="$(grep -E '^MAX_CONCURRENT_UPLOADS=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"
  existing_rate_limit="$(grep -E '^RATE_LIMIT_PER_MINUTE=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"

  [[ -n "${existing_port}" ]] && PORT="${PORT:-$existing_port}"
  [[ -n "${existing_data_dir}" ]] && DATA_DIR="${DATA_DIR:-$existing_data_dir}"
  [[ -n "${existing_public_base_url}" ]] && PUBLIC_BASE_URL="${PUBLIC_BASE_URL:-$existing_public_base_url}"
  [[ -n "${existing_max_concurrent}" ]] && MAX_CONCURRENT_UPLOADS="${MAX_CONCURRENT_UPLOADS:-$existing_max_concurrent}"
  [[ -n "${existing_rate_limit}" ]] && RATE_LIMIT_PER_MINUTE="${RATE_LIMIT_PER_MINUTE:-$existing_rate_limit}"
fi

if [[ "$INTERACTIVE" == "1" ]]; then
  DOMAIN="$(prompt_with_default "Domain (for nginx/public URL)" "${DOMAIN:-$DEFAULT_DOMAIN}")"
  PORT="$(prompt_with_default "Backend port (imgd listens on 0.0.0.0)" "${PORT:-$SUGGESTED_PORT}")"
  DATA_DIR="$(prompt_with_default "Image storage directory" "$DATA_DIR")"
  SERVICE_USER="$(prompt_with_default "Service user" "$SERVICE_USER")"
  MAX_CONCURRENT_UPLOADS="$(prompt_with_default "Max concurrent uploads" "$MAX_CONCURRENT_UPLOADS")"
  RATE_LIMIT_PER_MINUTE="$(prompt_with_default "Rate limit per IP per minute" "$RATE_LIMIT_PER_MINUTE")"
  PUBLIC_BASE_URL="$(prompt_with_default "PUBLIC_BASE_URL" "${PUBLIC_BASE_URL:-https://${DOMAIN}/images}")"
fi

if [[ -z "$DOMAIN" ]]; then
  DOMAIN="$DEFAULT_DOMAIN"
fi
if [[ -z "$PORT" ]]; then
  PORT="$SUGGESTED_PORT"
fi
if [[ -z "$PUBLIC_BASE_URL" ]]; then
  PUBLIC_BASE_URL="https://${DOMAIN}/images"
fi

if ! [[ "$PORT" =~ ^[0-9]+$ ]] || [[ "$PORT" -lt 1000 ]] || [[ "$PORT" -gt 65535 ]]; then
  fail "Invalid --port: $PORT (expected 1000-65535)"
fi

if [[ "$DOMAIN" == "0.0.0.0" ]]; then
  warn "Domain is 0.0.0.0. This is usually not externally reachable."
fi

if [[ $EUID -ne 0 ]]; then
  fail "Please run as root (use sudo)."
fi

if [[ ! -f "$BIN_PATH" ]]; then
  fail "Binary not found: $BIN_PATH"
fi

if [[ ! -x "$BIN_PATH" ]]; then
  chmod +x "$BIN_PATH"
fi

if ! command -v systemctl >/dev/null 2>&1; then
  fail "systemd is required"
fi

if [[ "$INTERACTIVE" == "1" ]]; then
  echo ""
  echo "Configuration summary:"
  echo "  Domain:               $DOMAIN"
  echo "  Port:                 $PORT"
  echo "  Data dir:             $DATA_DIR"
  echo "  Service user:         $SERVICE_USER"
  echo "  Public base URL:      $PUBLIC_BASE_URL"
  echo "  Max concurrent:       $MAX_CONCURRENT_UPLOADS"
  echo "  Rate limit/min:       $RATE_LIMIT_PER_MINUTE"
  echo "  Legacy UPLOAD_TOKEN:  $([[ -n \"$UPLOAD_TOKEN\" ]] && echo set || echo unset)"
  echo "  Configure nginx:      $([[ \"$SKIP_NGINX\" == \"0\" ]] && echo yes || echo no)"
  echo "  Enable/start service: $([[ \"$NO_ENABLE\" == \"0\" ]] && echo yes || echo no)"
  echo ""
  read -r -p "Continue install? [Y/n]: " confirm
  if [[ -n "$confirm" && ! "$confirm" =~ ^[Yy]$ ]]; then
    echo "Cancelled."
    exit 0
  fi
fi

if [[ "$SKIP_NGINX" == "0" ]]; then
  step "Checking nginx installation"
  if ! command -v nginx >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive
    step "Installing nginx"
    apt-get update
    apt-get install -y nginx
  fi
fi

if ! id -u "$SERVICE_USER" >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin "$SERVICE_USER"
fi

STATIC_GROUP="$SERVICE_USER"
if getent group www-data >/dev/null 2>&1; then
  STATIC_GROUP="www-data"
fi

install -d -m 755 /opt/imgd/bin /opt/imgd/conf
step "Installing imgd binary to /opt/imgd/bin/imgd"
install -m 755 "$BIN_PATH" /opt/imgd/bin/imgd

step "Preparing data directories under $DATA_DIR"
install -d -m 2750 -o "$SERVICE_USER" -g "$STATIC_GROUP" "$DATA_DIR"
install -d -m 700 -o "$SERVICE_USER" -g "$SERVICE_USER" "$DATA_DIR/.tmp"

step "Writing runtime environment file"
cat > /opt/imgd/conf/imgd.env <<ENV
PORT=${PORT}
PUBLIC_BASE_URL=${PUBLIC_BASE_URL}
DATA_DIR=${DATA_DIR}
TOKENS_FILE=/opt/imgd/conf/tokens.json
MAX_CONCURRENT_UPLOADS=${MAX_CONCURRENT_UPLOADS}
RATE_LIMIT_PER_MINUTE=${RATE_LIMIT_PER_MINUTE}
RUST_LOG=imgd=info,tower_http=info
ENV
if [[ -n "$UPLOAD_TOKEN" ]]; then
  echo "UPLOAD_TOKEN=${UPLOAD_TOKEN}" >> /opt/imgd/conf/imgd.env
fi
chmod 640 /opt/imgd/conf/imgd.env
chown root:"$SERVICE_USER" /opt/imgd/conf/imgd.env

TOKENS_FILE="/opt/imgd/conf/tokens.json"
if [[ ! -f "$TOKENS_FILE" ]]; then
  step "Initializing token store at $TOKENS_FILE"
  echo '{"tokens":[]}' > "$TOKENS_FILE"
  chmod 640 "$TOKENS_FILE"
  chown root:"$SERVICE_USER" "$TOKENS_FILE"
fi

step "Writing systemd unit: /etc/systemd/system/imgd.service"
cat > /etc/systemd/system/imgd.service <<SERVICE
[Unit]
Description=imgd upload service (axum)
After=network.target

[Service]
Type=simple
User=${SERVICE_USER}
Group=${SERVICE_USER}
WorkingDirectory=/opt/imgd
ExecStart=/opt/imgd/bin/imgd
EnvironmentFile=/opt/imgd/conf/imgd.env
Restart=always
RestartSec=3
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${DATA_DIR}
UMask=0027
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
SERVICE

if [[ "$SKIP_NGINX" == "0" ]]; then
  step "Detecting nginx config paths"
  discover_nginx_conf || fail "Cannot locate nginx.conf via nginx -V. Run with --skip-nginx and configure manually."
  NGINX_INCLUDE_DIR="$(pick_nginx_include_dir || true)"
  [[ -n "$NGINX_INCLUDE_DIR" ]] || fail "Cannot find included nginx config directory from $NGINX_CONF_PATH."
  NGINX_CONFIG_PATH="${NGINX_INCLUDE_DIR%/}/imgd.conf"
  step "Writing nginx server config to $NGINX_CONFIG_PATH"

  cat > "$NGINX_CONFIG_PATH" <<NGINX
server {
    listen 80;
    server_name ${DOMAIN};

    sendfile on;
    tcp_nopush on;
    etag on;

    types {
        image/webp webp;
    }
    default_type application/octet-stream;

    open_file_cache max=10000 inactive=60s;
    open_file_cache_valid 120s;
    open_file_cache_min_uses 2;
    open_file_cache_errors on;

    location /images/ {
        alias ${DATA_DIR}/;
        autoindex off;
        add_header Cache-Control "public, max-age=31536000, immutable" always;
        expires 365d;
        limit_except GET HEAD { deny all; }
        try_files \$uri =404;
    }

    location /upload {
        proxy_pass http://127.0.0.1:${PORT}/upload;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        client_max_body_size 6m;
    }

    location /healthz {
        proxy_pass http://127.0.0.1:${PORT}/healthz;
    }

    location /metrics {
        allow 127.0.0.1;
        deny all;
        proxy_pass http://127.0.0.1:${PORT}/metrics;
    }
}
NGINX

  step "Validating nginx config"
  nginx -t
fi

step "Reloading systemd daemon"
systemctl daemon-reload

tokens_count="$(grep -o '"token"[[:space:]]*:' "$TOKENS_FILE" | wc -l | tr -d ' ')"
CREATED_TOKEN=""
if [[ "${tokens_count}" == "0" && -z "$UPLOAD_TOKEN" ]]; then
  echo ""
  step "No token found. Creating initial token"
  token_name="default"
  token_days=""
  token_rate=""
  token_never="1"
  if [[ "$INTERACTIVE" == "1" ]]; then
    token_name="$(prompt_with_default "Token name" "default")"
    read -r -p "Token expires in N days (empty = never): " token_days
    token_rate="$(prompt_with_default "Per-token rate limit/min (empty = inherit-global)" "")"
    if [[ -n "$token_days" ]]; then
      token_never="0"
    fi
  fi

  token_cmd=(/opt/imgd/bin/imgd token create --name "$token_name" --tokens-file "$TOKENS_FILE")
  if [[ "$token_never" == "1" ]]; then
    token_cmd+=(--never-expire)
  else
    token_cmd+=(--days "$token_days")
  fi
  if [[ -n "$token_rate" ]]; then
    token_cmd+=(--rate-limit "$token_rate")
  fi
  token_output="$("${token_cmd[@]}")"
  echo "$token_output"
  CREATED_TOKEN="$(echo "$token_output" | awk -F': ' '/^token: /{print $2}' | tail -n1)"
fi

if [[ "$NO_ENABLE" == "0" ]]; then
  step "Enabling and starting imgd service"
  systemctl enable --now imgd
  if [[ "$SKIP_NGINX" == "0" ]]; then
    step "Reloading nginx"
    systemctl reload nginx
  fi
fi

echo ""
echo "${C_BOLD}Deploy complete.${C_RESET}"
echo "Service status:"
systemctl status imgd --no-pager || true
echo ""
echo "Health check (local):"
echo "  curl -i http://127.0.0.1:${PORT}/healthz"
echo "Upload API:"
if [[ -n "$CREATED_TOKEN" ]]; then
  echo "  curl -i -H \"X-Upload-Token: ${CREATED_TOKEN}\" -F \"file=@/path/to/a.webp\" http://${DOMAIN}/upload"
elif [[ -n "$UPLOAD_TOKEN" ]]; then
  echo "  curl -i -H \"X-Upload-Token: ${UPLOAD_TOKEN}\" -F \"file=@/path/to/a.webp\" http://${DOMAIN}/upload"
else
  echo "  create token first: /opt/imgd/bin/imgd token create --name default --never-expire --tokens-file ${TOKENS_FILE}"
fi
