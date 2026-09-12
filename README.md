# Pi Mobile UI

这是一个用 Rust 编写、为手机浏览器重新设计的 Pi Coding Agent 前端。它不是给原 `pi-web-ui` 换皮，而是一个独立服务：后端通过 Pi 的 JSONL RPC 模式驱动多个 session，前端只保留对话、输入、session 菜单和用户消息时间线。

## 已实现

- 新建、打开和切换 Pi session；session 仍保存在 Pi 原本的 `~/.pi/agent/sessions` 中。
- Pi 回复、思考内容与工具执行结果实时流式显示。
- 黑色半透明界面；顶部圆形加号与标题胶囊；菜单下拉时加号旋转成乘号。
- session 菜单底部有“新建 session”胶囊，按下才显示灰色反馈。
- 输入区域没有灰色框，只用一条横线和对话区分隔。
- 使用 `VisualViewport` 跟随手机输入法抬高输入区。
- 对话区双指下滑打开用户消息时间线，时间线双指上滑关闭；点击顶部标题也可打开时间线，方便不支持手势或手势冲突的浏览器。
- 默认仅监听 `127.0.0.1:3003`，适合放在 Caddy 与 Tailscale Funnel 后面。
- 单个 Rust 可执行文件内嵌全部网页资源，不需要 Node.js 运行时。

## 要求

- Linux 服务器（预编译文件为 x86_64 GNU/Linux）。
- 已安装可运行的 `pi`，并已配置模型/API 登录。
- 从源码构建需要 Rust stable。

先确认 Pi RPC 可启动：

```bash
pi --mode rpc
```

看到进程等待输入后按 `Ctrl+C` 退出即可。

## 快速运行

从源码：

```bash
cargo build --release
./target/release/pi-mobile-ui \
  --cwd /path/to/your/project \
  --pi-binary "$(command -v pi)"
```

或把随压缩包提供的 `pi-mobile-ui-x86_64-linux` 上传到服务器：

```bash
install -Dm755 pi-mobile-ui-x86_64-linux ~/.local/bin/pi-mobile-ui
~/.local/bin/pi-mobile-ui \
  --cwd /path/to/your/project \
  --pi-binary "$(command -v pi)"
```

然后在服务器本机检查：

```bash
curl http://127.0.0.1:3003/api/health
```

应返回 `{"ok":true,...}`。

## systemd

复制并编辑 `deploy/pi-mobile-ui.service.example`，把 `YOUR_USER`、`YOUR_PROJECT` 和 Pi 路径替换成服务器上的真实值：

```bash
sudo cp deploy/pi-mobile-ui.service.example /etc/systemd/system/pi-mobile-ui.service
sudoedit /etc/systemd/system/pi-mobile-ui.service
sudo systemctl daemon-reload
sudo systemctl enable --now pi-mobile-ui
sudo systemctl status pi-mobile-ui
```

如果 `pi` 是通过 npm/pnpm 安装的，`PI_MOBILE_PI` 必须填写 `command -v pi` 输出的绝对路径。systemd 不会读取你的交互式 shell 配置。

## Caddy 与 Tailscale Funnel

Caddy 的 `reverse_proxy` 原生支持 WebSocket，不需要单独配置升级头。保留你现有的域名、HTTPS、Basic Auth 和 Funnel 设置，只把旧 UI 的上游改成：

```caddyfile
reverse_proxy 127.0.0.1:3003
```

改完验证并平滑重载：

```bash
sudo caddy validate --config /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

不要把服务改成监听 `0.0.0.0`，也不要移除 Caddy 上现有的认证。这个面板能让 Pi 读写文件、执行命令，应视为服务器控制入口。

## 手机操作

- 点击左上角圆形加号：打开 session 菜单；加号会旋转成乘号。
- 点击 session：切换对话。
- 点击菜单底部“新建 session”：创建一个真正独立的 Pi session。
- 输入框按 `Ctrl/Command + Enter` 也可发送；回复中发送按钮会变成停止按钮。
- 在对话区两指同步下滑：打开全部用户消息时间线。
- 在时间线两指同步上滑：退出时间线。
- 点击顶部 session 标题：无手势备用入口。

浏览器会比较两指中点的垂直位移，并排除明显的缩放动作。iOS Safari 与 Android Chrome 都支持所用的 Touch Events；系统级缩放或页面滚动仍可能抢占边缘手势，因此保留了标题点击入口。

## 配置

命令行参数均有同名环境变量：

| 参数 | 环境变量 | 默认值 |
| --- | --- | --- |
| `--host` | `PI_MOBILE_HOST` | `127.0.0.1` |
| `--port` | `PI_MOBILE_PORT` | `3003` |
| `--cwd` | `PI_MOBILE_CWD` | 启动目录 |
| `--pi-binary` | `PI_MOBILE_PI` | `pi` |
| `--agent-dir` | `PI_CODING_AGENT_DIR` | `~/.pi/agent` |
| `--max-sessions` | `PI_MOBILE_MAX_SESSIONS` | `8` |

## 验证

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
node tests/smoke.mjs
```

冒烟测试使用假的 Pi RPC 进程，不调用模型，也不会消耗 API 配额。它覆盖 HTTP 静态资源、WebSocket、新建 session、发送消息和流式回复。

## 当前边界

- 为保持界面克制，首版没有模型选择器、附件上传、session 删除和分支编辑器。
- 同时存活的 Pi RPC 进程默认最多 8 个；重启服务会释放进程，但不会删除任何 session 文件。
- 时间线包含 JSONL 中所有用户消息（包括压缩前历史和废弃分支）；只有当前活动分支中可见的消息能直接滚动定位。

RPC 实现依据 Pi 官方的 `pi --mode rpc` JSONL 协议；旧版 Pi 不支持 `get_entries` 时会自动退回到当前消息列表生成时间线。

