# Cancelling a run asks first

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
┏ Workflows 2 · Runs 2 ━━━━━━━━━━━━━━━━━━━━━━━━━━┓╭ Detail ──────────────────────────────────────────────────╮
┃▸ Implement, review, ship  Fix flaky checkout…  ┃│Implement, review, ship  awaiting-gate                    │
┃▸ Weekly security digest  Migrate auth to pas…  ┃│started by Mara 30m ago · driven by This device           │
┃  Implement, review, ship ●●◆ awaiting-gate 4m  ┃│input inject a fake clock                                 │
┃  Weekly security digest ● completed 1d         ┃│                                                          │
┃                                                ┃│  ● Implement @coder  succeeded                           │
┃                                                ┃│      Changed the checkout struct and the timeout check.  │
┃                                                ┃│  ● Review @reviewer  succeeded                           │
┃                                                ┃│      Looks right; the timeout has a test.                │
┃                                                ┃│  ◆ Sign-off gate (1 approval)  awaiting-approval         │
┃                                                ┃│                                                          │
┃                                                ┃│A approve the gate  R reject it                           │
┃                      ┏ Cancel the run ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓                      │
┃                      ┃Cancel the run “Implement, review, ship”? Its stages stop and ┃                      │
┃                      ┃it can't be resumed.                                          ┃                      │
┃                      ┃                                                              ┃                      │
┃                      ┃y cancel the run · any other key backs out                    ┃                      │
┃                      ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛                      │
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
↑↓ select  S start  A approve gate  R reject  X cancel  ? help       peer to peer · lt on this terminal · demo
```
