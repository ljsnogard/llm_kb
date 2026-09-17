#!/usr/bin/env bash
# iceoryx2 v0.9.3 可行性 PoC 驱动脚本。
#
# 需要在一个 bash 调用内跑完（宿主的 /tmp 不跨调用保留）：
#   bash external/iceoryx2-poc/run-poc.sh 2>&1 | tee external/iceoryx2-poc/poc-output.txt
#
# 输出被设计成「机读友好」，会被抄进 dev-notes。

set -u

ROOT="$(cd "$(dirname "$0")" && pwd)"
# 宿主的 /tmp 不跨调用保留，因此把 CARGO_HOME 放在工作区的 external/cargo-home
# （根 .gitignore 已忽略该目录），这样重复跑 PoC 不必重新下载依赖。
export CARGO_HOME="${CARGO_HOME:-$ROOT/../cargo-home}"
mkdir -p "$CARGO_HOME"
[ -f "$CARGO_HOME/config.toml" ] || cp /root/.cargo/config.toml "$CARGO_HOME/" 2>/dev/null || true

BIN="$ROOT/target/debug/ipc-poc"

section() { printf '\n===== %s =====\n' "$1"; }

section "env"
rustc --version
cargo --version
echo "uname: $(uname -a)"
echo "shm dir: $(ls -ld /dev/shm 2>/dev/null || echo 'NO /dev/shm')"
echo "XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-<unset>}"

section "cargo build (依赖下载 + 编译)"
cargo build --manifest-path "$ROOT/Cargo.toml" 2>&1 | tail -5
BUILD_EXIT="${PIPESTATUS[0]}"
echo "build exit: $BUILD_EXIT"
if [ "$BUILD_EXIT" -ne 0 ]; then
  echo "!!! 构建失败，后续 e2e 会命中上一轮的陈旧二进制，因此直接中止 !!!"
  cargo build --manifest-path "$ROOT/Cargo.toml" 2>&1 | grep -E '^error' -A 6 | head -60
  exit 1
fi

section "iceoryx2 依赖规模"
echo "cargo tree 行数: $(cargo tree --manifest-path "$ROOT/Cargo.toml" 2>/dev/null | wc -l)"
echo "去重 crate 数: $(cargo tree --manifest-path "$ROOT/Cargo.toml" --prefix none 2>/dev/null | sort -u | wc -l)"

# ── 1. 跨进程 pub/sub ────────────────────────────────────────────────────
section "e2e pub/sub (两个进程, 定长结构体载荷)"
"$BIN" sub > /tmp/sub.log 2>&1 &
SUB_PID=$!
sleep 0.5
"$BIN" pub > /tmp/pub.log 2>&1
wait $SUB_PID
echo "--- pub.log ---"; cat /tmp/pub.log
echo "--- sub.log ---"; cat /tmp/sub.log

# ── 2. 跨进程 request/response ──────────────────────────────────────────
section "e2e request/response (两个进程)"
"$BIN" srv > /tmp/srv.log 2>&1 &
SRV_PID=$!
sleep 0.5
"$BIN" cli > /tmp/cli.log 2>&1
echo "--- cli.log ---"; cat /tmp/cli.log
sleep 0.2
kill $SRV_PID 2>/dev/null
wait $SRV_PID 2>/dev/null
echo "--- srv.log ---"; cat /tmp/srv.log

# ── 3. 4 MiB 单次载荷（切片载荷路径） ────────────────────────────────────
section "e2e 4MiB 单次载荷 (切片载荷 + write_from_fn)"
"$BIN" big-sub > /tmp/bigsub.log 2>&1 &
BIG_PID=$!
sleep 0.5
"$BIN" big-pub > /tmp/bigpub.log 2>&1
wait $BIG_PID
echo "--- big-pub.log ---"; cat /tmp/bigpub.log
echo "--- big-sub.log ---"; cat /tmp/bigsub.log

# ── 3b. 4 MiB：不配置切片上限（默认策略）的失败模式 ─────────────────────
section "e2e 4MiB 不配置 initial_max_slice_len (预期 LoanError)"
"$BIN" big-sub > /tmp/bigsub_d.log 2>&1 &
BIGD_PID=$!
sleep 0.5
"$BIN" big-pub-default > /tmp/bigpub_d.log 2>&1
kill $BIGD_PID 2>/dev/null; wait $BIGD_PID 2>/dev/null
echo "--- big-pub-default.log ---"; cat /tmp/bigpub_d.log

# ── 3c. 4 MiB：Static 预分配 ────────────────────────────────────────────
section "e2e 4MiB initial_max_slice_len(BIG_CAP) + AllocationStrategy::Static"
"$BIN" big-sub > /tmp/bigsub_s.log 2>&1 &
BIGS_PID=$!
sleep 0.5
"$BIN" big-pub-static > /tmp/bigpub_s.log 2>&1
wait $BIGS_PID
echo "--- big-pub-static.log ---"; cat /tmp/bigpub_s.log
echo "--- big-sub.log ---"; cat /tmp/bigsub_s.log

# ── 4. 变长载荷（同一服务混合长度） ─────────────────────────────────────
section "e2e 变长载荷 (13B / 60KB / 512KB 混合)"
"$BIN" var-sub > /tmp/varsub.log 2>&1 &
VAR_PID=$!
sleep 0.5
"$BIN" var-pub > /tmp/varpub.log 2>&1
wait $VAR_PID
echo "--- var-pub.log ---"; cat /tmp/varpub.log
echo "--- var-sub.log ---"; cat /tmp/varsub.log

# ── 5. 二次启动：进程全退出后再跑一遍 ───────────────────────────────────
section "e2e 二次启动 (验证无残留导致的失败)"
"$BIN" sub > /tmp/sub2.log 2>&1 &
SUB2_PID=$!
sleep 0.5
"$BIN" pub > /tmp/pub2.log 2>&1
wait $SUB2_PID
echo "--- sub2.log ---"; cat /tmp/sub2.log

# ── 5b. 崩溃恢复：SIGKILL 掉发布者后再跑一遍 ────────────────────────────
section "e2e 崩溃恢复 (SIGKILL 发布者后再通信)"
"$BIN" pub > /tmp/pubkill.log 2>&1 &
KILL_PID=$!
sleep 0.3
kill -9 $KILL_PID 2>/dev/null
wait $KILL_PID 2>/dev/null
echo "killed publisher pid=$KILL_PID (SIGKILL)"
"$BIN" sub > /tmp/sub3.log 2>&1 &
SUB3_PID=$!
sleep 0.5
"$BIN" pub > /tmp/pub3.log 2>&1
wait $SUB3_PID
echo "--- sub3.log ---"; cat /tmp/sub3.log

# ── 6. 线程安全端点 ─────────────────────────────────────────────────────
section "threadsafe (ipc_threadsafe::Service 的 Send/Sync)"
"$ROOT/target/debug/threadsafe" 2>&1 | tail -20

# ── 7. 反向用例：预期编译失败 ───────────────────────────────────────────
section "negative compile cases (预期每个都失败)"
for case in string_payload vec_payload nested_string enum_payload option_payload move_across_thread send_required; do
  printf '\n--- neg/%s ---\n' "$case"
  out=$(cargo build --manifest-path "$ROOT/neg/Cargo.toml" --bin "$case" 2>&1)
  code=$?
  echo "exit: $code"
  if [ "$code" -eq 0 ]; then
    echo "RESULT: UNEXPECTED-COMPILE-SUCCESS"
  else
    echo "$out" | grep -E '^error(\[|:)|^  = note|^help:' | head -12
  fi
done

# ── 8. 直接读 iceoryx2 源码，确认配置与线程语义 ─────────────────────────
section "iceoryx2 源码：Global 配置字段"
SRC="$(ls -d "$CARGO_HOME"/registry/src/*/iceoryx2-0.9.3 2>/dev/null | head -1)"
echo "src dir: $SRC"
if [ -n "$SRC" ]; then
  sed -n '/^pub struct Global {/,/^}/p' "$SRC/src/config.rs" | head -60
  printf '\n--- Defaults (默认 QoS) 摘要 ---\n'
  sed -n '/^pub struct Defaults {/,/^}/p' "$SRC/src/config.rs" | head -40
  printf '\n--- ZeroCopySend trait 定义 ---\n'
  grep -rn -A 12 'pub unsafe trait ZeroCopySend' "$SRC/../iceoryx2-bb-elementary-traits-0.9.3/src/zero_copy_send.rs" 2>/dev/null | head -30
  printf '\n--- ipc / ipc_threadsafe 模块文档 ---\n'
  sed -n '1,40p' "$SRC/src/service/ipc.rs" 2>/dev/null
  sed -n '1,40p' "$SRC/src/service/ipc_threadsafe.rs" 2>/dev/null
fi

# ── 9. 遗留资源 ─────────────────────────────────────────────────────────
section "resource leftovers"
echo "--- /dev/shm ---"; ls -la /dev/shm 2>/dev/null | head -20
echo "--- /tmp/iceoryx2 (size) ---"; du -sh /tmp/iceoryx2 2>/dev/null; find /tmp/iceoryx2 -maxdepth 2 2>/dev/null | head -20

section "DONE"
