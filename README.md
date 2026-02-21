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

## Nginx 配置

示例文件：`deploy/nginx/imgd.conf`

关键点：
- `location /images/ { alias /data/images/; }`
- `autoindex off`
- 强缓存：`Cache-Control: public, max-age=31536000, immutable`
- `sendfile on`
- `open_file_cache` 提升静态性能
- 开启 `etag on`

## systemd 配置

示例文件：`deploy/systemd/imgd.service`

关键点：
- 独立用户运行：`User=imgd`
- 自动重启：`Restart=always`
- 环境变量注入：`PORT/UPLOAD_TOKEN/PUBLIC_BASE_URL/DATA_DIR`
- `ReadWritePaths=/data/images` 限制写入范围

## 权限建议

- Rust 服务用户 `imgd`：对 `/data/images` 有写权限
- Nginx：对 `/data/images` 只有读权限
- 临时文件目录：`/data/images/.tmp`（建议 `750`）

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
