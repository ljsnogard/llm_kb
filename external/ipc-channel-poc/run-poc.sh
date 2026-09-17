#!/usr/bin/env bash
# servo/ipc-channel 0.23 可行性 PoC 驱动脚本。
#
# 需要在一次 bash 调用内跑完（宿主 /tmp 不跨调用保留）：
#   bash external/ipc-channel-poc/run-poc.sh 2>&1 | tee external/ipc-channel-poc/poc-output.txt

set -u

ROOT="$(cd "$(dirname "$0")" && pwd)"
export CARGO_HOME="${CARGO_HOME:-$ROOT/../cargo-home}"
mkdir -p "$CARGO_HOME"
[ -f "$CARGO_HOME/config.toml" ] || cp /root/.cargo/config.toml "$CARGO_HOME/" 2>/dev/null || true

BIN="$ROOT/target/debug/ipc-poc"
NAME_FILE="$ROOT/target/ipc-name.txt"

section() { printf '\n===== %s =====\n' "$1"; }

# 启动一个 one-shot server，等它把名字写进 NAME_FILE，再跑客户端。
run_pair() {
  local server_mode="$1"
  local client_mode="$2"
  rm -f "$NAME_FILE"
  "$BIN" "$server_mode" "$NAME_FILE" > "$ROOT/target/$server_mode.log" 2>&1 &
  local srv_pid=$!
  for _ in $(seq 1 100); do
    [ -s "$NAME_FILE" ] && break
    sleep 0.05
  done
  echo "one-shot name: $(cat "$NAME_FILE" 2>/dev/null)"
  "$BIN" "$client_mode" "$NAME_FILE" 2>&1
  wait "$srv_pid"
  cat "$ROOT/target/$server_mode.log"
}

section "env"
rustc --version
uname -a

section "cargo build"
cargo build --manifest-path "$ROOT/Cargo.toml" 2>&1 | tail -4
BUILD_EXIT="${PIPESTATUS[0]}"
echo "build exit: $BUILD_EXIT"
if [ "$BUILD_EXIT" -ne 0 ]; then
  cargo build --manifest-path "$ROOT/Cargo.toml" 2>&1 | grep -E '^error' -A 8 | head -80
  exit 1
fi

section "依赖规模"
echo "cargo tree 行数: $(cargo tree --manifest-path "$ROOT/Cargo.toml" 2>/dev/null | wc -l)"
echo "去重 crate 数: $(cargo tree --manifest-path "$ROOT/Cargo.toml" --prefix none 2>/dev/null | sort -u | wc -l)"

section "1. typed 通道：String / Vec<String> / Vec<u8> 直发（跨进程，3 次往返）"
run_pair typed-server typed-client

section "2. bytes_channel：完全不用 serde 的原始字节通路（含 4 MiB 单条）"
run_pair bytes-server bytes-client

section "3. IpcSharedMemory：4 MiB 只传句柄，消息类型零用户 serde"
run_pair shm-server shm-client

section "4. 异步：IpcReceiver::to_stream() 在 tokio 多线程下消费 500 条"
"$BIN" stream-demo 2>&1

section "5. 对比：内联 recv() 饿死运行时 vs to_stream() 不阻塞"
"$BIN" blocking-demo 2>&1

section "遗留资源"
echo "--- one-shot socket 文件 ---"
ls -la "$ROOT/target"/ipc-name.txt 2>/dev/null
echo "--- 系统临时目录里的 ipc-channel socket ---"
ls -la /tmp/ipc-channel* 2>/dev/null | head -10 || true

section "DONE"
