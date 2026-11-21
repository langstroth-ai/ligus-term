# Agents: nothing found on this device

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
┏ Agents ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃Agent        Status   Runtime      Runs on            Who can use            Doing                          ┃
┃@primary     idle     claude-code  This device        Everyone               answers untagged messages      ┃
┃@coder       busy     claude-code  sff · online       Only me                working 40%: wiring the CSV exp┃
┃@reviewer    idle     codex        This device        Everyone               reviews diffs                  ┃
┃@scout       idle     ollama       sff · online       Everyone                                              ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
╭ Available on this device ──────────────────────────────────────────────────────────────────────────────────╮
│No agent runtime found on this device.                                                                      │
│Install Claude Code, Codex, the GitHub Copilot CLI or the Antigravity CLI (agy), start Ollama, or set       │
│ANTHROPIC_API_KEY / OPENAI_API_KEY / OPENROUTER_API_KEY, then open this tab again.                          │
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Usage on this device ──────────────────────────────────────────────────────────────────────────────────────╮
│Usage is shown to the agent's owner, on the device that runs it.                                            │
│                                                                                                            │
│                                                                                                            │
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Hand-offs ─────────────────────────────────────────────────────────────────────────────────────────────────╮
│@coder → @reviewer review the fake-clock change 20m                                                         │
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Runtimes ──────────────────────────────────────────────────────────────────────────────────────────────────╮
│Runtime       Kind                     Model                              Where         Status              │
│claude-code   Claude Code              default                            local CLI     Configured · Active │
│codex         Codex                    default                            local CLI     Configured          │
│ollama        Ollama (local)           qwen3-coder:30b                    local server  Configured          │
│copilot       GitHub Copilot CLI       default                            local CLI     Not configured      │
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
↑↓ agents  N add agent  r runtime  E who can use  ? help             peer to peer · lt on this terminal · demo
```
