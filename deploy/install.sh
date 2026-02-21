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
  --token <upload_token>       Upload token for API auth
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
    --token 'replace-with-long-random-token' \
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

generate_token() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 24
    return
  fi
  head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n'
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
DEFAULT_TOKEN="$(generate_token)"

if [[ -f "$EXISTING_ENV_FILE" ]]; then
  existing_port="$(grep -E '^PORT=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"
  existing_token="$(grep -E '^UPLOAD_TOKEN=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"
  existing_data_dir="$(grep -E '^DATA_DIR=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"
  existing_public_base_url="$(grep -E '^PUBLIC_BASE_URL=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"
  existing_max_concurrent="$(grep -E '^MAX_CONCURRENT_UPLOADS=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"
  existing_rate_limit="$(grep -E '^RATE_LIMIT_PER_MINUTE=' "$EXISTING_ENV_FILE" | head -n1 | cut -d= -f2- || true)"

  [[ -n "${existing_port}" ]] && PORT="${PORT:-$existing_port}"
  [[ -n "${existing_token}" ]] && UPLOAD_TOKEN="${UPLOAD_TOKEN:-$existing_token}"
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
  UPLOAD_TOKEN="$(prompt_with_default "Upload token" "${UPLOAD_TOKEN:-$DEFAULT_TOKEN}")"
  PUBLIC_BASE_URL="$(prompt_with_default "PUBLIC_BASE_URL" "${PUBLIC_BASE_URL:-https://${DOMAIN}/images}")"
fi

if [[ -z "$DOMAIN" ]]; then
  DOMAIN="$DEFAULT_DOMAIN"
fi
if [[ -z "$PORT" ]]; then
  PORT="$SUGGESTED_PORT"
fi
if [[ -z "$UPLOAD_TOKEN" ]]; then
  UPLOAD_TOKEN="$DEFAULT_TOKEN"
fi
if [[ -z "$PUBLIC_BASE_URL" ]]; then
  PUBLIC_BASE_URL="https://${DOMAIN}/images"
fi

if ! [[ "$PORT" =~ ^[0-9]+$ ]] || [[ "$PORT" -lt 1000 ]] || [[ "$PORT" -gt 65535 ]]; then
  echo "Invalid --port: $PORT (expected 1000-65535)" >&2
  exit 1
fi

if [[ $EUID -ne 0 ]]; then
  echo "Please run as root (use sudo)." >&2
  exit 1
fi

if [[ ! -f "$BIN_PATH" ]]; then
  echo "Binary not found: $BIN_PATH" >&2
  exit 1
fi

if [[ ! -x "$BIN_PATH" ]]; then
  chmod +x "$BIN_PATH"
fi

if ! command -v systemctl >/dev/null 2>&1; then
  echo "systemd is required" >&2
  exit 1
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
  if ! command -v nginx >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive
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
install -m 755 "$BIN_PATH" /opt/imgd/bin/imgd

install -d -m 2750 -o "$SERVICE_USER" -g "$STATIC_GROUP" "$DATA_DIR"
install -d -m 700 -o "$SERVICE_USER" -g "$SERVICE_USER" "$DATA_DIR/.tmp"

cat > /opt/imgd/conf/imgd.env <<ENV
PORT=${PORT}
UPLOAD_TOKEN=${UPLOAD_TOKEN}
PUBLIC_BASE_URL=${PUBLIC_BASE_URL}
DATA_DIR=${DATA_DIR}
TOKENS_FILE=/opt/imgd/conf/tokens.json
MAX_CONCURRENT_UPLOADS=${MAX_CONCURRENT_UPLOADS}
RATE_LIMIT_PER_MINUTE=${RATE_LIMIT_PER_MINUTE}
RUST_LOG=imgd=info,tower_http=info
ENV
chmod 640 /opt/imgd/conf/imgd.env
chown root:"$SERVICE_USER" /opt/imgd/conf/imgd.env

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
  cat > /etc/nginx/sites-available/imgd.conf <<NGINX
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

  ln -sf /etc/nginx/sites-available/imgd.conf /etc/nginx/sites-enabled/imgd.conf
  nginx -t
fi

systemctl daemon-reload

if [[ "$NO_ENABLE" == "0" ]]; then
  systemctl enable --now imgd
  if [[ "$SKIP_NGINX" == "0" ]]; then
    systemctl reload nginx
  fi
fi

echo ""
echo "Deploy complete."
echo "Service status:"
systemctl status imgd --no-pager || true
echo ""
echo "Health check (local):"
echo "  curl -i http://127.0.0.1:${PORT}/healthz"
echo "Upload API:"
echo "  curl -i -H \"X-Upload-Token: ${UPLOAD_TOKEN}\" -F \"file=@/path/to/a.webp\" http://${DOMAIN}/upload"
