# Workflows: definitions and runs

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
┏ Workflows 2 · Runs 2 ━━━━━━━━━━━━━━━━━━━━━━━━━━┓╭ Detail ──────────────────────────────────────────────────╮
┃▸ Implement, review, ship  Fix flaky checkout…  ┃│Implement, review, ship                                   │
┃▸ Weekly security digest  Migrate auth to pas…  ┃│in Fix flaky checkout test                                │
┃  Implement, review, ship ●●◆ awaiting-gate 4m  ┃│                                                          │
┃  Weekly security digest ● completed 1d         ┃│The coder implements, the reviewer checks it, a person    │
┃                                                ┃│signs off.                                                │
┃                                                ┃│                                                          │
┃                                                ┃│  Implement @coder                                        │
┃                                                ┃│  Review @reviewer  after implement                       │
┃                                                ┃│  Sign-off gate (1 approval)  after review                │
┃                                                ┃│                                                          │
┃                                                ┃│S Start a run: this device drives it; stages go to their  │
┃                                                ┃│agents' hosts                                             │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┃                                                ┃│                                                          │
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛╰──────────────────────────────────────────────────────────╯
↑↓ select  S start  A approve gate  R reject  X cancel  ? help                 peer to peer · 1 running · demo
```
