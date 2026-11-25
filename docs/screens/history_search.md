# ctrl-r in the composer: what you sent

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
╭ Chats ───────────────────────╮╭ Fix flaky checkout test · 1/3 ─────────────────────────────────────────────╮
│  ◆ Fix flaky checkout … 12m  ││  ◆ proposal “Inject the clock into checkout t…” · open 1/2 · p to review   │
│  ◆ Migrate auth to pas… 50m  ││                                                                            │
│  Weekly release notes    1d  ││Ada  12m                                                                    │
│                              ││  @Mara can you review the timeout? 30 seconds is what the provider uses.   │
│                              ││  ◆ proposal “Sign-off: inject a fake clock” · open 0/1 · p to review       │
│                              ││                                                                            │
│                              ││Mara  now                                                                   │
│                              ││  @coder can you rerun the flaky test?                                      │
│                              ││                                                                            │
│                              ││@coder  via claude-code · on build-box  now                                 │
│                              ││  ▸ read_file src/checkout.rs done 0.2s                                     │
│                              ││  Here's what I found: the checkout test reads the wall clock, so it fails  │
│                              ││  when CI is slow. Want me to propose a change?                             │
│                              ││                                                                            │
│                              ││Mara  now                                                                   │
│                              ││  Thanks, that fixed it                                                     │
│                              ││                                                                            │
│                              ││@ligus  via claude-code · on this device  now                               │
│                              ││  ▸ read_file src/checkout.rs done 0.2s                                     │
│                              ││  Looked at the checkout test. I injected a fake clock into the session     │
│                ┏ What you sent · enter use · esc ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓                │
│                ┃search ›                                                                  ┃                │
│                ┃                                                                          ┃                │
│                ┃▶ Thanks, that fixed it                                                   ┃· p to review   │
│                ┃  @coder can you rerun the flaky test?                                    ┃────────────────╯
│                ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
│                              │┏ Message the room ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
│                              │┃Type a message · @ mentions an agent or a person · esc for keys             ┃
╰──────────────────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
type to search  ↑↓ select  enter use  esc close                                peer to peer · 1 running · demo
```
