# Set up an agent: ready to run

Rendered by `tests/setup.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
┏ Chats ━━━━━━━━━━━━━━━━━━━━━━━┓╭ Fix flaky checkout test · 1/3 ─────────────────────────────────────────────╮
┃  ◆ Fix flaky checkout … 12m  ┃│  The checkout test fails about one run in ten. Can we stop it reading the  │
┃  ◆ Migrate auth to pas… 50m  ┃│  wall clock?                                                               │
┃  Weekly release notes    1d  ┃│                                                                            │
┃                              ┃│@coder  via claude-code · on build-box  38m                                 │
┃                              ┃│  Yes. The payment tests already use a fake clock, so it's a small change.  │
┃                              ┃│  I'll propose it.                                                          │
┃                              ┃│                                                                            │
┃                              ┃│  ✓ Ada answered Fake clock · 37m                                           │
┃                ┏ Set up an agent ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓· p to review   │
┃                ┃Ready to run                                                              ┃                │
┃                ┃  Harness          Claude Code                                            ┃                │
┃                ┃  Name             helper                                                 ┃rovider uses.   │
┃                ┃  Model            sonnet                                                 ┃                │
┃                ┃  Run a test turn  yes                                                    ┃res?”           │
┃                ┃                                                                          ┃e chat first)   │
┃                ┃[ ] dry run  checks and validates, writes nothing                         ┃                │
┃                ┃                                                                          ┃                │
┃                ┃enter run · d dry run · shift-tab back · esc cancel                       ┃                │
┃                ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛                │
┃                              ┃│    │ cargo test -p checkout --features fake-clock                          │
┃                              ┃│    in /srv/northwind                                                       │
┃                              ┃│    coder describes it as: Run the checkout tests                           │
┃                              ┃│    Next, after the question above                                          │
┃                              ┃│  ◆ proposal “Sign-off: inject a fake clock” · open 0/1 · p to review       │
┃                              ┃╰────────────────────────────────────────────────────────────────────────────╯
┃                              ┃
┃                              ┃╭ Message the room ──────────────────────────────────────────────────────────╮
┃                              ┃│i to write · @ mentions an agent or a person                                │
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛╰────────────────────────────────────────────────────────────────────────────╯
i write  N new chat  tab pane  . actions  : commands  ? help                   peer to peer · 1 running · demo
```
