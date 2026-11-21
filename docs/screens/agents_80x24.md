# agents at 80×24

Rendered by `tests/screens.rs` (80×24, the in-memory demo backend).

```text
[AT]+1• │ Chats Flows Props Agents Hosts Memory People Notifs 1 Setup  Apiary T…
┏ Agents · a taller terminal shows usage ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃Agent    Status   Runtime      Runs on           Who can use      Doing       ┃
┃@primary idle     claude-code  This device       Everyone         answers unta┃
┃@coder   busy     claude-code  sff · online      Only me          working 40%:┃
┃@reviewe idle     codex        This device       Everyone         reviews diff┃
┃@scout   idle     ollama       sff · online      Everyone                     ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
╭ Available on this device · N or enter adds an agent ─────────────────────────╮
│Runtime                  Model            Version   Status     Detail         │
│GitHub Copilot CLI       default          1.0.106   not ready  no Copilot CLI │
│Ollama (local)           qwen3:8b                   ready      Ollama at 127.0│
╰──────────────────────────────────────────────────────────────────────────────╯
╭ Hand-offs ───────────────────────────────────────────────────────────────────╮
│@coder → @reviewer review the mites-per-100 change 20m                        │
╰──────────────────────────────────────────────────────────────────────────────╯
╭ Runtimes ────────────────────────────────────────────────────────────────────╮
│Runtime       Kind           Model          Where         Status              │
│claude-code   Claude Code    default        local CLI     Configured · Active │
│codex         Codex          default        local CLI     Configured          │
│ollama        Ollama (local) qwen3-coder:30 local server  Configured          │
│copilot       GitHub Copilot default        local CLI     Not configured      │
╰──────────────────────────────────────────────────────────────────────────────╯
↑↓ agents  N add agent  r runtime  E who can use  ? help            peer to peer
```
