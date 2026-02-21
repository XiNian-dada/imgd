docker run --rm -t \
  --platform=linux/amd64 \
  -v "$(pwd)":/work \
  -v "$HOME/.cargo/registry":/usr/local/cargo/registry \
  -v "$HOME/.cargo/git":/usr/local/cargo/git \
  -w /work \
  rust:latest \
  cargo build --release
