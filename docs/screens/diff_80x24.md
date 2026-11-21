# diff at 80×24

Rendered by `tests/screens.rs` (80×24, the in-memory demo backend).

```text
[AT]+1• │ Chats Flows Props Agents Hosts Memory People Notifs 1 Setup  Apiary T…
╭ Proposals ───────────────────╮╭ Record mites per 100 bees ───────────────────╮
│open     Record mites per …   ││open  by @coder · 2 files changed · 1/2       │
│         1/2 approvals · @c…  ││approvals                                     │
│approved Export inspection…   ││✓ Ada approved · admin                        │
│         2/2 approvals · @c…  ││A Approve   R Reject   O Apply on sff… (needs │
│applied  Bump the frame sc…   ││approval first)                               │
│         2/2 approvals · @r…  ││That host applies it once, on its checkout,   │
│open     Sign-off: count m…   │╰──────────────────────────────────────────────╯
│         0/1 approvals · wo…  │┏ Diff · 2 files · hunk 1/3 ━━━━━━━━━━━━━━━━━━━┓
│open     Workflow: Swarm c…   │┃▌src/inspections.rs +9 −3                     ┃
│         0/1 approvals · @s…  │┃@@ -12,7 +12,9 @@ pub struct Inspection {     ┃
│                              │┃     pub colony: ColonyId,                    ┃
│                              │┃     pub frames: Vec<FrameNote>,              ┃
│                              │┃-    pub mites: u32,                          ┃
│                              │┃+    /// Mites per 100 bees, from an alcohol …┃
│                              │┃+    pub mites_per_100: f32,                  ┃
│                              │┃+    pub wash_sample: u32,                    ┃
│                              │┃     pub queen_seen: bool,                    ┃
│                              │┃ }                                            ┃
│                              │┃@@ -40,9 +42,13 @@ impl Inspection {          ┃
│                              │┃     pub fn needs_treatment(&self) -> bool {  ┃
╰──────────────────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
↑↓ scroll  ] next hunk  [ prev hunk  A approve  R reject  O apply  ? help
```
