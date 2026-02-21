# imgd

一个自用的极简图床上传后端：
- Nginx 负责静态文件直出
- Rust (axum + tokio) 只负责上传 API

## 功能

- `POST /upload`（`multipart/form-data`，字段名必须为 `file`）
- 鉴权：
  - `X-Upload-Token: <token>`
  - 或 `Authorization: Bearer <token>`
- 仅允许 WebP：
  - 扩展名必须是 `.webp`
  - 文件头必须匹配 `RIFF....WEBP`
- 上传大小限制：单文件最大 5MB（流式读取过程中截断）
- 流式写盘：避免整文件读入内存
- 内容哈希命名：`/data/images/YYYY/MM/<sha256>.webp`
- 去重：同内容二次上传不重复落盘
- 基础抗滥用：
  - 每 IP 简易限流
  - 并发上传闸门（Semaphore）
- 可观测：
  - tracing 结构化日志（ip/request_id/sha256/size/elapsed/result）
  - `GET /healthz` 返回 `ok`
  - `GET /metrics` 返回计数（成功/失败/限流）

## 环境变量

- `PORT`：监听端口，默认 `3000`
- `UPLOAD_TOKEN`：上传 token（必填）
- `PUBLIC_BASE_URL`：公网图片前缀，例如 `https://img.example.com/images`（必填）
- `DATA_DIR`：图片根目录，默认 `/data/images`
- `MAX_CONCURRENT_UPLOADS`：最大并发上传数，默认 `16`
- `RATE_LIMIT_PER_MINUTE`：每 IP 每分钟请求上限，默认 `60`
- `RUST_LOG`：日志级别（例如 `imgd=info,tower_http=info`）

## 本地运行

```bash
PORT=3000 \
UPLOAD_TOKEN=secret \
PUBLIC_BASE_URL=https://img.example.com/images \
DATA_DIR=/data/images \
cargo run
```

启动时会自动：
- 创建 `DATA_DIR`
- 创建 `DATA_DIR/.tmp`
- 检查写权限（不可写直接启动失败）

## API

### `POST /upload`

请求：
- `multipart/form-data`
- 字段名：`file`

成功响应：

```json
{
  "url": "https://img.example.com/images/2026/02/<sha256>.webp",
  "path": "/2026/02/<sha256>.webp",
  "sha256": "...",
  "size": 12345
}
```

失败响应格式：

```json
{
  "error": "unauthorized|unsupported_media_type|file_too_large|bad_request|too_many_requests|internal_error"
}
```

## 手工验收

### 1) 健康检查

```bash
curl http://127.0.0.1:3000/healthz
```

### 2) 上传一张 WebP

```bash
curl -H "X-Upload-Token: secret" \
  -F "file=@/path/to/a.webp" \
  http://127.0.0.1:3000/upload
```

### 3) 重复上传同一文件（验证去重）

再次上传同文件，返回 `sha256/path` 应一致。

### 4) 伪装 WebP（应 415）

```bash
printf 'hello' > /tmp/fake.webp
curl -H "X-Upload-Token: secret" \
  -F "file=@/tmp/fake.webp" \
  http://127.0.0.1:3000/upload
```

### 5) 超大文件（应 413）

上传 >5MB 文件，返回 `file_too_large`。

### 6) 连续请求（应触发 429）

```bash
seq 1 100 | xargs -P 32 -I{} \
  curl -s -o /dev/null -w "%{http_code}\n" \
  -H "X-Upload-Token: secret" \
  -F "file=@/path/to/a.webp" \
  http://127.0.0.1:3000/upload | sort | uniq -c
```

## 生产部署（Ubuntu x86_64）

你说得对，部署应该是“二进制到服务器后，全在服务器执行”。

已提供一键脚本：`deploy/install.sh`（交互式）。

### 1) 准备文件（在服务器上）

确保服务器上有这两个文件：
- `imgd`（Linux x86_64 二进制）
- `deploy/install.sh`

给执行权限：

```bash
chmod +x ./imgd ./deploy/install.sh
```

### 2) 一键部署（在服务器上）

```bash
sudo ./deploy/install.sh --bin ./imgd
```

脚本会逐项询问：
- 域名
- 端口
- 存储目录
- systemd 服务用户
- token 等关键配置

直接回车会使用默认值，其中端口默认是“随机可用四位数端口”。

脚本会自动完成：
- 创建/复用系统用户 `imgd`
- 安装并配置 Nginx（可选）
- 安装二进制到 `/opt/imgd/bin/imgd`
- 生成 `/opt/imgd/conf/imgd.env`
- 创建 systemd 服务 `/etc/systemd/system/imgd.service`
- 创建静态目录 `/data/images` 与临时目录 `/data/images/.tmp`
- 启动并设置开机自启 `imgd`
- 重载 Nginx

### 3) 验收

```bash
PORT=$(grep '^PORT=' /opt/imgd/conf/imgd.env | cut -d= -f2)
curl -i "http://127.0.0.1:${PORT}/healthz"
curl -i -H "X-Upload-Token: replace-with-long-random-token" \
  -F "file=@/path/to/a.webp" \
  http://img.example.com/upload
```

### 4) 常用参数

```bash
# 自定义端口、数据目录、限流
sudo ./deploy/install.sh \
  --domain img.example.com \
  --token 'xxx' \
  --bin ./imgd \
  --port 3000 \
  --data-dir /data/images \
  --max-concurrent 16 \
  --rate-limit 60

# 只部署后端，不改 Nginx
sudo ./deploy/install.sh --domain img.example.com --token 'xxx' --bin ./imgd --skip-nginx

# 仅写配置，不立即启动
sudo ./deploy/install.sh --domain img.example.com --token 'xxx' --bin ./imgd --no-enable

# 禁用交互（CI/脚本场景）
sudo ./deploy/install.sh --non-interactive --domain img.example.com --token 'xxx' --bin ./imgd
```

### 5) 更新版本

把新二进制覆盖到服务器后，重复执行同一条安装命令即可（幂等）。

### 6) 日志与状态

```bash
sudo systemctl status imgd --no-pager
sudo journalctl -u imgd -f
```

### 7) Token 管理（新增）

服务支持通过命令生成多个 token，并可单独控制过期时间与频率限制。

```bash
# 生成一个 token：30 天过期，每分钟最多 120 次上传
/opt/imgd/bin/imgd token create --name mobile --days 30 --rate-limit 120 --tokens-file /opt/imgd/conf/tokens.json

# 生成一个永不过期 token（不设独立限流，继承全局 RATE_LIMIT_PER_MINUTE）
/opt/imgd/bin/imgd token create --name ci --never-expire --tokens-file /opt/imgd/conf/tokens.json

# 按 RFC3339 指定过期时间
/opt/imgd/bin/imgd token create --name temp --expires-at 2026-12-31T23:59:59Z --rate-limit 30 --tokens-file /opt/imgd/conf/tokens.json

# 查看 token（展示 name/过期/限流/token_id，不直接泄露 token）
/opt/imgd/bin/imgd token list --tokens-file /opt/imgd/conf/tokens.json

# 撤销 token（按 name 或原始 token）
/opt/imgd/bin/imgd token revoke --name mobile --tokens-file /opt/imgd/conf/tokens.json
```

修改 token 后重启服务生效：

```bash
sudo systemctl restart imgd
```

## 国内 crates 镜像（已配置清华源）

项目内 `.cargo/config.toml`：

```toml
[source.crates-io]
replace-with = "tuna"

[source.tuna]
registry = "sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/"
```

如你更偏好阿里源，可改为：

```toml
registry = "sparse+https://mirrors.aliyun.com/crates.io-index/"
```
