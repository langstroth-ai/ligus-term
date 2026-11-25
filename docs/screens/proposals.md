# Proposals with the diff viewer

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
╭ Proposals ───────────────────────────────╮╭ Inject the clock into checkout tests ──────────────────────────╮
│Inject the clock into c…  open 1/2 · 37m  ││open  by @coder · 2 files changed · 1/2 approvals               │
│Export orders to CSV   approved 2/2 · 1d  ││✓ Ada approved · admin                                          │
│Bump the order schema…  applied 2/2 · 2d  │╰────────────────────────────────────────────────────────────────╯
│Sign-off: inject a fake …  open 0/1 · 4m  │┏ Diff · 2 files · hunk 1/3 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
│Workflow: Dependency au…  open 0/1 · 20m  │┃▌src/checkout.rs +9 −3                                          ┃
│                                          │┃@@ -12,7 +12,9 @@ pub struct Checkout {                         ┃
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
│                                          │┃     pub fn summary(&self) -> String {                          ┃
│                                          │┃                                                                ┃
│                                          │┃▌tests/checkout.rs +8 −0                                        ┃
│                                          │┃@@ -0,0 +1,8 @@                                                 ┃
╰──────────────────────────────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
] next hunk  [ prev hunk  A approve  . actions  : commands  ? help             peer to peer · 1 running · demo
```
