# Set up: the guided flows and what is done

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
┃                ┏ Set up ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓                │
┃                ┃› Set up an agent    ✓ 4 agents                                           ┃                │
┃                ┃  Connect a tool       no tools connected                                 ┃· p to review   │
┃                ┃  Add a code folder    no repositories yet                                ┃                │
┃                ┃  Add a box            run it on the box                                  ┃                │
┃                ┃  Add a phone          no phone paired                                    ┃rovider uses.   │
┃                ┃  Invite a person      no invites yet                                     ┃                │
┃                ┃  Set up a relay     ✓ https://relay.langstroth.ai                        ┃res?”           │
┃                ┃  Add a computer       not linked to your devices                         ┃e chat first)   │
┃                ┃                                                                          ┃                │
┃                ┃An agent answers when you @mention it. It runs one harness (Claude Code,  ┃                │
┃                ┃Codex, Copilot, Antigravity, Aider, Pi or a local model) inside this      ┃                │
┃                ┃device's agent sandbox.                                                   ┃                │
┃                ┃                                                                          ┃                │
┃                ┃enter start · esc close · every flow is safe to run again                 ┃                │
┃                ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛                │
┃                              ┃│    Next, after the question above                                          │
┃                              ┃│  ◆ proposal “Sign-off: inject a fake clock” · open 0/1 · p to review       │
┃                              ┃╰────────────────────────────────────────────────────────────────────────────╯
┃                              ┃
┃                              ┃╭ Message the room ──────────────────────────────────────────────────────────╮
┃                              ┃│i to write · @ mentions an agent or a person                                │
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛╰────────────────────────────────────────────────────────────────────────────╯
i write  N new chat  tab pane  . actions  : commands  ? help                   peer to peer · 1 running · demo
```
