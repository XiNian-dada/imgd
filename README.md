# imgd

Minimal image-host upload backend:
- `imgd` handles upload API only (`/upload`)
- Nginx serves static files (`/images/...`)

---

## English

### 1) Quick Deploy (on server)

Download and extract `imgd-linux-amd64.zip` from GitHub Release, then run:

```bash
chmod +x ./imgd ./install.sh
sudo ./install.sh --bin ./imgd
```

After install:

```bash
sudo systemctl status imgd --no-pager
PORT=$(grep '^PORT=' /opt/imgd/conf/imgd.env | cut -d= -f2)
curl -i "http://127.0.0.1:${PORT}/healthz"
```

### 2) What `install.sh` does

The installer is interactive (press Enter to accept defaults). It performs these steps:

1. Checks Nginx availability.
What it means: if Nginx is missing and you did not use `--skip-nginx`, it installs Nginx.

2. Installs binary to `/opt/imgd/bin/imgd`.
What it means: this is the managed runtime path used by systemd.

3. Prepares storage directories.
What it means: creates image root and temporary upload dir, and applies ownership/permissions.

4. Writes runtime config `/opt/imgd/conf/imgd.env`.
What it means: stores `PORT`, `PUBLIC_BASE_URL`, `DATA_DIR`, `TOKENS_FILE`, and limits.

5. Initializes token store `/opt/imgd/conf/tokens.json`.
What it means: multi-token auth source used by the service.

6. Writes systemd unit `/etc/systemd/system/imgd.service`.
What it means: service is managed by `systemctl` (start/restart/enable/logs).

7. Detects real Nginx config layout from `nginx -V` and `nginx.conf` includes.
What it means: supports package Nginx and panel/custom Nginx layouts.

8. Writes `imgd` Nginx server config to detected include directory.
What it means: enables `/images/` static mapping and `/upload` reverse proxy.

9. Validates Nginx (`nginx -t`) and applies reload.
What it means: config must pass syntax check before going live.

10. Optionally creates initial token (if none) and starts service.
What it means: service becomes usable immediately.

### 3) Create Tokens

```bash
# Never-expire token
/opt/imgd/bin/imgd token create --name default --never-expire --tokens-file /opt/imgd/conf/tokens.json

# 30-day token with per-token limit
/opt/imgd/bin/imgd token create --name mobile --days 30 --rate-limit 120 --tokens-file /opt/imgd/conf/tokens.json

# List tokens
/opt/imgd/bin/imgd token list --tokens-file /opt/imgd/conf/tokens.json

# Revoke token
/opt/imgd/bin/imgd token revoke --name mobile --tokens-file /opt/imgd/conf/tokens.json
```

Apply token changes:

```bash
sudo systemctl restart imgd
```

### 4) Upload Test

```bash
curl -i \
  -H "X-Upload-Token: <your-token>" \
  -F "file=@/path/to/1.webp" \
  http://<your-domain>/upload
```

Expected success fields: `url`, `path`, `sha256`, `size`.

### 5) Common Issues

#### A) `sendfile directive is not allowed here .../tcp/imgd.conf`
Cause: an old/incorrect config was written under TCP/stream include path.

Fix:

```bash
sudo rm -f /path/to/nginx/tcp/imgd.conf
sudo nginx -t && sudo nginx -s reload
```

#### B) Upload succeeds but image returns 404
Checklist:

```bash
# 1) file exists
ls -l /data/images/YYYY/MM/<sha256>.webp

# 2) force host match
curl -I -H "Host: <your-domain>" http://127.0.0.1/images/YYYY/MM/<sha256>.webp

# 3) confirm nginx user
grep -n '^user' <nginx.conf>

# 4) directory traversal/read permissions
sudo chmod 755 /data
sudo find /data/images -type d -exec chmod 750 {} \;
sudo find /data/images -type f -exec chmod 640 {} \;

# 5) reload
sudo nginx -t && sudo nginx -s reload
```

#### C) returned URL contains `0.0.0.0`
Cause: `PUBLIC_BASE_URL` is incorrect.

Fix:

```bash
sudo sed -i 's|^PUBLIC_BASE_URL=.*|PUBLIC_BASE_URL=http://<your-domain>/images|' /opt/imgd/conf/imgd.env
sudo systemctl restart imgd
```

#### D) HTTPS certificate mismatch
Cause: certificate does not include your image domain.

Fix:
- Issue/bind a valid certificate for your domain.
- Use `http://` as temporary `PUBLIC_BASE_URL` until TLS is fixed.

### 6) CI/CD Artifacts

- Actions artifact: `imgd-linux-amd64.zip`
- Release asset: `imgd-linux-amd64.zip`
- `main` updates `latest` prerelease, `v*` tags publish versioned release

---

## 中文

### 1) 快速部署（在服务器上）

从 GitHub Release 下载并解压 `imgd-linux-amd64.zip`，执行：

```bash
chmod +x ./imgd ./install.sh
sudo ./install.sh --bin ./imgd
```

安装后检查：

```bash
sudo systemctl status imgd --no-pager
PORT=$(grep '^PORT=' /opt/imgd/conf/imgd.env | cut -d= -f2)
curl -i "http://127.0.0.1:${PORT}/healthz"
```

### 2) `install.sh` 每一步在做什么

脚本是交互式的（直接回车用默认值），核心步骤如下：

1. 检查 Nginx 是否可用。
含义：若未安装且未加 `--skip-nginx`，会自动安装。

2. 把二进制安装到 `/opt/imgd/bin/imgd`。
含义：systemd 固定从这里启动服务。

3. 创建并初始化存储目录。
含义：准备图片目录和临时上传目录，并设置权限。

4. 写入 `/opt/imgd/conf/imgd.env`。
含义：写入端口、公开 URL、数据目录、token 文件、限流参数等。

5. 初始化 `/opt/imgd/conf/tokens.json`。
含义：多 token 鉴权的数据来源。

6. 生成 systemd 服务文件。
含义：后续可用 `systemctl` 管理（启动/重启/开机自启/日志）。

7. 通过 `nginx -V` + `nginx.conf include` 自动识别 Nginx 配置布局。
含义：兼容系统包安装和面板/自编译路径。

8. 把 `imgd` 的 Nginx 配置写到正确 include 目录。
含义：启用 `/images/` 静态和 `/upload` 反代。

9. 执行 `nginx -t` 校验并重载。
含义：配置无语法错误才会生效。

10. 若无 token，则引导创建首个 token，并启动服务。
含义：安装完成即可直接上传。

### 3) 创建 Token

```bash
# 永不过期
/opt/imgd/bin/imgd token create --name default --never-expire --tokens-file /opt/imgd/conf/tokens.json

# 30 天过期 + 每分钟 120 次
/opt/imgd/bin/imgd token create --name mobile --days 30 --rate-limit 120 --tokens-file /opt/imgd/conf/tokens.json

# 查看
/opt/imgd/bin/imgd token list --tokens-file /opt/imgd/conf/tokens.json

# 吊销
/opt/imgd/bin/imgd token revoke --name mobile --tokens-file /opt/imgd/conf/tokens.json
```

修改 token 后：

```bash
sudo systemctl restart imgd
```

### 4) 上传测试

```bash
curl -i \
  -H "X-Upload-Token: <你的token>" \
  -F "file=@/path/to/1.webp" \
  http://<你的域名>/upload
```

成功返回字段：`url`、`path`、`sha256`、`size`。

### 5) 常见问题

#### A) `sendfile directive is not allowed here .../tcp/imgd.conf`
原因：错误配置曾写入了 tcp/stream 目录。

处理：

```bash
sudo rm -f /path/to/nginx/tcp/imgd.conf
sudo nginx -t && sudo nginx -s reload
```

#### B) 上传成功但图片 404
排查顺序：

```bash
# 1) 文件是否存在
ls -l /data/images/YYYY/MM/<sha256>.webp

# 2) 强制 Host 命中对应站点
curl -I -H "Host: <你的域名>" http://127.0.0.1/images/YYYY/MM/<sha256>.webp

# 3) 查看 nginx 运行用户
grep -n '^user' <nginx.conf>

# 4) 确保可遍历/可读
sudo chmod 755 /data
sudo find /data/images -type d -exec chmod 750 {} \;
sudo find /data/images -type f -exec chmod 640 {} \;

# 5) 重载
sudo nginx -t && sudo nginx -s reload
```

#### C) 返回 URL 是 `0.0.0.0`
原因：`PUBLIC_BASE_URL` 配错。

处理：

```bash
sudo sed -i 's|^PUBLIC_BASE_URL=.*|PUBLIC_BASE_URL=http://<你的域名>/images|' /opt/imgd/conf/imgd.env
sudo systemctl restart imgd
```

#### D) HTTPS 证书不匹配
原因：证书没有覆盖你的图片域名。

处理：
- 给该域名签发并绑定正确证书
- 证书修好前先用 `http://` 作为 `PUBLIC_BASE_URL`

### 6) CI/CD 产物

- Actions 工件：`imgd-linux-amd64.zip`
- Release 资产：`imgd-linux-amd64.zip`
- `main` 自动更新 `latest` 预发布，`v*` tag 发布版本 Release
