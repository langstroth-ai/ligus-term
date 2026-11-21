# agents at 80×24

Rendered by `tests/screens.rs` (80×24, the in-memory demo backend).

```text
[NA]+1• │ Chats Flows Props Agents Hosts Memory People Notifs 1 Setup  Northwin…
┏ Agents · a taller terminal shows usage ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃Agent      Status Runtime    Runs on         Who can use         Doing        ┃
┃@primary   idle   claude-cod This device     Everyone            answers untag┃
┃@coder     busy   claude-cod sff · online    Only me             working 40%: ┃
┃@reviewer  idle   codex      This device     Everyone            reviews diffs┃
┃@scout     idle   ollama     sff · online    Everyone                         ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
╭ Available on this device · N or enter adds an agent ─────────────────────────╮
│Runtime                  Model            Version   Status     Detail         │
│GitHub Copilot CLI       default          1.0.106   not ready  no Copilot CLI │
│Ollama (local)           qwen3:8b                   ready      Ollama at 127.0│
╰──────────────────────────────────────────────────────────────────────────────╯
╭ Hand-offs ───────────────────────────────────────────────────────────────────╮
│@coder → @reviewer review the fake-clock change 20m                           │
╰──────────────────────────────────────────────────────────────────────────────╯
╭ Runtimes ────────────────────────────────────────────────────────────────────╮
│Runtime     Kind                  Model          Where       Status           │
│claude-code Claude Code           default        local CLI   Configured · Acti│
│codex       Codex                 default        local CLI   Configured       │
│ollama      Ollama (local)        qwen3-coder:30 local serve Configured       │
│copilot     GitHub Copilot CLI    default        local CLI   Not configured   │
╰──────────────────────────────────────────────────────────────────────────────╯
↑↓ agents  N add agent  r runtime  E who can use  ? help            peer to peer
```
