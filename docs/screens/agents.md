# Agents and runtimes

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[AT] [QR]• Apiary Tracker · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
┏ Agents ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃Agent        Status   Runtime      Runs on            Who can use            Doing                          ┃
┃@primary     idle     claude-code  This device        Everyone               answers untagged messages      ┃
┃@coder       busy     claude-code  sff · online       Only me                working 40%: wiring the CSV exp┃
┃@reviewer    idle     codex        This device        Everyone               reviews diffs                  ┃
┃@scout       idle     ollama       sff · online       Everyone                                              ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
╭ Usage on this device ──────────────────────────────────────────────────────────────────────────────────────╮
│Usage is shown to the agent's owner, on the device that runs it.                                            │
│                                                                                                            │
│                                                                                                            │
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
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
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
↑↓ agents  r runtime  E who can use  ? help                                        lt on this terminal · demo
```
