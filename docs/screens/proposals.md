# Proposals with the diff viewer

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[AT] [QR]• Apiary Tracker · owner │ Chats  Proposals  Agents  Hosts  People  Notifications 1  Settings
╭ Proposals ───────────────────────────────╮╭ Record mites per 100 bees ─────────────────────────────────────╮
│open     Record mites per 100 bees        ││open  by @coder · 2 files changed · 1/2 approvals               │
│         1/2 approvals · @coder · 37m     ││✓ Ada approved · admin                                          │
│approved Export inspections to CSV        ││A Approve   R Reject   O Apply on sff… (needs approval first)   │
│         2/2 approvals · @coder · 1d      ││That host applies it once, on its checkout, and reports the     │
│applied  Bump the frame schema version    ││result.                                                         │
│         2/2 approvals · @reviewer · 2d   ││                                                                │
│                                          │╰────────────────────────────────────────────────────────────────╯
│                                          │┏ Diff · 2 files · hunk 1/3 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
│                                          │┃▌src/inspections.rs +9 −3                                       ┃
│                                          │┃@@ -12,7 +12,9 @@ pub struct Inspection {                       ┃
│                                          │┃     pub colony: ColonyId,                                      ┃
│                                          │┃     pub frames: Vec<FrameNote>,                                ┃
│                                          │┃-    pub mites: u32,                                            ┃
│                                          │┃+    /// Mites per 100 bees, from an alcohol wash.              ┃
│                                          │┃+    pub mites_per_100: f32,                                    ┃
│                                          │┃+    pub wash_sample: u32,                                      ┃
│                                          │┃     pub queen_seen: bool,                                      ┃
│                                          │┃ }                                                              ┃
│                                          │┃@@ -40,9 +42,13 @@ impl Inspection {                            ┃
│                                          │┃     pub fn needs_treatment(&self) -> bool {                    ┃
│                                          │┃-        self.mites > 9                                         ┃
│                                          │┃-    }                                                          ┃
│                                          │┃+        // 3 per 100 is the usual action threshold in late sum…┃
│                                          │┃+        self.mites_per_100 >= TREATMENT_THRESHOLD              ┃
│                                          │┃+    }                                                          ┃
│                                          │┃+                                                               ┃
│                                          │┃+    pub const TREATMENT_THRESHOLD: f32 = 3.0;                  ┃
│                                          │┃                                                                ┃
╰──────────────────────────────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
↑↓ scroll  ] next hunk  [ prev hunk  A approve  R reject  O apply                  lt on this terminal · demo
```
