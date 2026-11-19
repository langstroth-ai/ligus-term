# Agents and runtimes

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[AT] [QR]• Apiary Tracker · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
┏ Agents ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃Agent        Status   Runtime      Runs on            Doing                                                 ┃
┃@primary     idle     claude-code  This device        answers untagged messages                             ┃
┃@coder       busy     claude-code  sff · online       working 40%: wiring the CSV export                    ┃
┃@reviewer    idle     codex        This device        reviews diffs                                         ┃
┃@scout       idle     ollama       sff · online                                                             ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
╭ Hand-offs ─────────────────────────────────────────────────────────────────────────────────────────────────╮
│@coder → @reviewer review the mites-per-100 change 20m                                                      │
│                                                                                                            │
│                                                                                                            │
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Runtimes ──────────────────────────────────────────────────────────────────────────────────────────────────╮
│Runtime       Kind                     Model                              Where         Status              │
│claude-code   Claude Code              default                            local CLI     Configured · Active │
│codex         Codex                    default                            local CLI     Configured          │
│ollama        Ollama (local)           qwen3-coder:30b                    local server  Configured          │
│copilot       GitHub Copilot CLI       default                            local CLI     Not configured      │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
↑↓ agents  r runtime  ? help                                                       lt on this terminal · demo
```
