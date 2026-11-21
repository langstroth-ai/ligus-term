# Proposals with the diff viewer

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
╭ Proposals ───────────────────────────────╮╭ Inject the clock into checkout tests ──────────────────────────╮
│open     Inject the clock into checkou…   ││open  by @coder · 2 files changed · 1/2 approvals               │
│         1/2 approvals · @coder · 37m     ││✓ Ada approved · admin                                          │
│approved Export orders to CSV             ││A Approve   R Reject   O Apply on sff… (needs approval first)   │
│         2/2 approvals · @coder · 1d      ││That host applies it once, on its checkout, and reports the     │
│applied  Bump the order schema version    ││result.                                                         │
│         2/2 approvals · @reviewer · 2d   ││                                                                │
│open     Sign-off: inject a fake clock    │╰────────────────────────────────────────────────────────────────╯
│         0/1 approvals · workflow · 4m    │┏ Diff · 2 files · hunk 1/3 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
│open     Workflow: Dependency audit be…   │┃▌src/checkout.rs +9 −3                                          ┃
│         0/1 approvals · @scout · 20m     │┃@@ -12,7 +12,9 @@ pub struct Checkout {                         ┃
│                                          │┃     pub cart: CartId,                                          ┃
│                                          │┃     pub items: Vec<LineItem>,                                  ┃
│                                          │┃-    pub started: Instant,                                      ┃
│                                          │┃+    /// Injected, so tests set the time instead of reading the…┃
│                                          │┃+    pub clock: Arc<dyn Clock>,                                 ┃
│                                          │┃+    pub started_at: Millis,                                    ┃
│                                          │┃     pub paid: bool,                                            ┃
│                                          │┃ }                                                              ┃
│                                          │┃@@ -40,9 +42,13 @@ impl Checkout {                              ┃
│                                          │┃     pub fn is_expired(&self) -> bool {                         ┃
│                                          │┃-        self.started.elapsed().as_secs() > 30                  ┃
│                                          │┃-    }                                                          ┃
│                                          │┃+        // 30 seconds matches the payment provider's session t…┃
│                                          │┃+        self.clock.now() - self.started_at >= Self::SESSION_TI…┃
│                                          │┃+    }                                                          ┃
│                                          │┃+                                                               ┃
│                                          │┃+    pub const SESSION_TIMEOUT_MS: Millis = 30_000;             ┃
│                                          │┃                                                                ┃
╰──────────────────────────────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
↑↓ scroll  ] next hunk  [ prev hunk  A approve  R reject  O apply  ? help                         peer to peer
```
