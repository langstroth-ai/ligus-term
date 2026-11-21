# diff at 80×24

Rendered by `tests/screens.rs` (80×24, the in-memory demo backend).

```text
[NA]+1• │ Chats Flows Props Agents Hosts Memory People Notifs 1 Setup  Northwin…
╭ Proposals ───────────────────╮╭ Inject the clock into checkout tests ────────╮
│open     Inject the clock …   ││open  by @coder · 2 files changed · 1/2       │
│         1/2 approvals · @c…  ││approvals                                     │
│approved Export orders to …   ││✓ Ada approved · admin                        │
│         2/2 approvals · @c…  ││A Approve   R Reject   O Apply on sff… (needs │
│applied  Bump the order sc…   ││approval first)                               │
│         2/2 approvals · @r…  ││That host applies it once, on its checkout,   │
│open     Sign-off: inject …   │╰──────────────────────────────────────────────╯
│         0/1 approvals · wo…  │┏ Diff · 2 files · hunk 1/3 ━━━━━━━━━━━━━━━━━━━┓
│open     Workflow: Depende…   │┃▌src/checkout.rs +9 −3                        ┃
│         0/1 approvals · @s…  │┃@@ -12,7 +12,9 @@ pub struct Checkout {       ┃
│                              │┃     pub cart: CartId,                        ┃
│                              │┃     pub items: Vec<LineItem>,                ┃
│                              │┃-    pub started: Instant,                    ┃
│                              │┃+    /// Injected, so tests set the time inst…┃
│                              │┃+    pub clock: Arc<dyn Clock>,               ┃
│                              │┃+    pub started_at: Millis,                  ┃
│                              │┃     pub paid: bool,                          ┃
│                              │┃ }                                            ┃
│                              │┃@@ -40,9 +42,13 @@ impl Checkout {            ┃
│                              │┃     pub fn is_expired(&self) -> bool {       ┃
╰──────────────────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
↑↓ scroll  ] next hunk  [ prev hunk  A approve  R reject  O apply  ? help
```
