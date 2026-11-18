# Agents and runtimes

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[AT] [QR]• Apiary Tracker · owner │ Chats  Proposals  Agents  Hosts  People  Notifications 1  Settings
┏ Agents ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃Agent          Runtime       Role                                                Runs on          Scope     ┃
┃@primary       claude-code   answers untagged messages                           This device      workspace ┃
┃@coder         claude-code   implements proposals                                sff · online     workspace ┃
┃@reviewer      codex         reviews diffs                                       This device      workspace ┃
┃@scout         ollama        agent                                               sff · online     workspace ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
╭ Runtimes ──────────────────────────────────────────────────────────────────────────────────────────────────╮
│Runtime       Kind                     Model                              Where         Status              │
│claude-code   Claude Code              default                            local CLI     Configured · Active │
│codex         Codex                    default                            local CLI     Configured          │
│ollama        Ollama (local)           qwen3-coder:30b                    local server  Configured          │
│copilot       GitHub Copilot CLI       default                            local CLI     Not configured      │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
│                                                                                                            │
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
↑↓ agents  r runtime  ? help                                                       lt on this terminal · demo
```
