# / in a list: the selection jumps to the match

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
┏ Proposals ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓╭ Export orders to CSV ──────────────────────────────────────────╮
┃open     Inject the clock into checkou…   ┃│approved  by @coder · 1 file changed · 2/2 approvals            │
┃         1/2 approvals · @coder · 37m     ┃│✓ Mara approved · owner                                         │
┃approved Export orders to CSV             ┃│✓ Ada approved · admin                                          │
┃         2/2 approvals · @coder · 1d      ┃│A Approve  R Reject (already approved)   O Apply on sff…        │
┃applied  Bump the order schema version    ┃│That host applies it once, on its checkout, and reports the     │
┃         2/2 approvals · @reviewer · 2d   ┃│result.                                                         │
┃open     Sign-off: inject a fake clock    ┃│                                                                │
┃         0/1 approvals · workflow · 4m    ┃╰────────────────────────────────────────────────────────────────╯
┃open     Workflow: Dependency audit be…   ┃╭ Diff · 1 file · hunk 1/1 ──────────────────────────────────────╮
┃         0/1 approvals · @scout · 20m     ┃│▌src/export.rs +4 −0                                            │
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
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛╰────────────────────────────────────────────────────────────────╯
/csv  1 match  ↑↓ next  enter keep  esc back                                   peer to peer · 1 running · demo
```
