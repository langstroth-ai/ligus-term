# / in a list: the selection jumps to the match

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
┏ Proposals ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓╭ Export orders to CSV ──────────────────────────────────────────╮
┃Inject the clock into c…  open 1/2 · 37m  ┃│approved  by @coder · 1 file changed · 2/2 approvals            │
┃Export orders to CSV   approved 2/2 · 1d  ┃│✓ Mara approved · owner                                         │
┃Bump the order schema…  applied 2/2 · 2d  ┃│✓ Ada approved · admin                                          │
┃Sign-off: inject a fake …  open 0/1 · 4m  ┃╰────────────────────────────────────────────────────────────────╯
┃Workflow: Dependency au…  open 0/1 · 20m  ┃╭ Diff · 1 file · hunk 1/1 ──────────────────────────────────────╮
┃                                          ┃│▌src/export.rs +4 −0                                            │
┃                                          ┃│@@ -1,3 +1,7 @@                                                 │
┃                                          ┃│ use crate::Order;                                              │
┃                                          ┃│+pub fn to_csv(rows: &[Order]) -> String {                      │
┃                                          ┃│+    rows.iter().map(Order::csv_row).collect::<Vec<_>>().join("…│
┃                                          ┃│+}                                                              │
┃                                          ┃│+                                                               │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┃                                          ┃│                                                                │
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛╰────────────────────────────────────────────────────────────────╯
/csv  1 match  ↑↓ next  enter keep  esc back                                   peer to peer · 1 running · demo
```
