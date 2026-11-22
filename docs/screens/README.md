# Screens

Text captures of every `lt` screen, rendered by the snapshot tests in
`tests/screens.rs` into a ratatui `TestBackend` (110×32 unless the file
says otherwise) against the
in-memory demo backend. Colors aren't captured here; the dark, light and
mono themes render the same text, except the Ligus mark on the first-run
and empty screens, which mono draws as a silhouette.

Regenerate after a UI change:

```sh
INSTA_UPDATE=always LT_WRITE_SCREENS=1 cargo test --test screens
```

| Screen | File |
|---|---|
| First run: create or join | [onboarding.md](onboarding.md) |
| First run in a narrow terminal: no mark | [onboarding_narrow.md](onboarding_narrow.md) |
| First run in a narrow, tall terminal: the mark above the form | [onboarding_narrow_tall.md](onboarding_narrow_tall.md) |
| First run in the mono theme: the mark as a silhouette | [onboarding_mono.md](onboarding_mono.md) |
| First run: join with an invite | [onboarding_join.md](onboarding_join.md) |
| Join: who the invite is from, checked before joining | [onboarding_join_checked.md](onboarding_join_checked.md) |
| First run: connect a deployment (ligus-local, Kubernetes) | [onboarding_connect.md](onboarding_connect.md) |
| First run: a validation error | [onboarding_error.md](onboarding_error.md) |
| Creating a workspace asks for a passkey | [passkey_create.md](passkey_create.md) |
| The recovery key, shown once | [recovery_key.md](recovery_key.md) |
| A new workspace | [new_workspace.md](new_workspace.md) |
| A new workspace in a short terminal: the welcome without the mark | [new_workspace_short.md](new_workspace_short.md) |
| Start-up: the cursor in the composer | [start.md](start.md) |
| Chats: list and transcript | [chat.md](chat.md) |
| An agent streaming its reply | [chat_streaming.md](chat_streaming.md) |
| Composer with the @mention popup | [chat_mentions.md](chat_mentions.md) |
| Multi-line composer | [chat_multiline.md](chat_multiline.md) |
| A locked-down chat | [chat_lockdown.md](chat_lockdown.md) |
| Workspace switcher | [switcher.md](switcher.md) |
| Create or join another workspace | [add_workspace.md](add_workspace.md) |
| Workflows: definitions and runs | [workflows.md](workflows.md) |
| A run waiting on its gate | [workflow_run_gate.md](workflow_run_gate.md) |
| The gate approved: the run completes | [workflow_run_done.md](workflow_run_done.md) |
| Starting a run | [workflow_start.md](workflow_start.md) |
| A run started here, live | [workflow_run_live.md](workflow_run_live.md) |
| Proposals with the diff viewer | [proposals.md](proposals.md) |
| Diff viewer: a later hunk | [proposals_hunk.md](proposals_hunk.md) |
| Apply on ‹host› | [apply_host.md](apply_host.md) |
| Agents (status, what they're doing), runtimes, hand-offs | [agents.md](agents.md) |
| Hosts and workers | [hosts.md](hosts.md) |
| Binding this device's checkout for apply on host | [bind_checkout.md](bind_checkout.md) |
| Hosts with a bound checkout | [hosts_bound.md](hosts_bound.md) |
| Task state and memory | [memory.md](memory.md) |
| A memory entry | [memory_entry.md](memory_entry.md) |
| People: roles and actions | [people.md](people.md) |
| People: refused actions with the reason | [people_refused.md](people_refused.md) |
| Changing a role | [role_picker.md](role_picker.md) |
| Confirm a removal | [confirm_remove.md](confirm_remove.md) |
| Quitting while this device drives a run | [confirm_quit.md](confirm_quit.md) |
| Cancelling a run asks first | [confirm_cancel_run.md](confirm_cancel_run.md) |
| Admit a box: its join request | [admit_box.md](admit_box.md) |
| Admit a box: the sealed admission | [admit_box_sealed.md](admit_box_sealed.md) |
| Passkey sheet | [presence.md](presence.md) |
| Passkey sheet: recovery-words file | [presence_recovery.md](presence_recovery.md) |
| Passkey sheet with no browser here: the address for another device | [presence_no_browser.md](presence_no_browser.md) |
| Notifications | [notifications.md](notifications.md) |
| Settings | [settings.md](settings.md) |
| Help: keybindings | [help.md](help.md) |
| Help: `/` narrows the keys to what matches | [help_search.md](help_search.md) |
| `space`, then a pause: the keys you can press here | [which_key.md](which_key.md) |
| `ctrl-r` in the composer: what you sent | [history_search.md](history_search.md) |
| Command palette | [palette.md](palette.md), typing filters: [palette_query.md](palette_query.md) |
| Chats: failed, paused and working chips | [chat_states.md](chat_states.md) |
| Messages: this session's notices and errors | [messages.md](messages.md), empty: [messages_empty.md](messages_empty.md) |
| Status bar: offline, with an error toast in its centre | [status_offline_toast.md](status_offline_toast.md) |

SVG images of a few screens, in color, are in
[assets/screens](../../assets/screens), from `cargo run --example screenshots`.

## At 80×24

The default terminal size. `tests/screens.rs` also checks every main view at
60×20 and 200×50 (snapshots in `tests/snapshots/`): the top bar shortens its
labels, then shows one workspace tile and the view digits, instead of cutting
views off; the status bar keeps the help key and drops hints from the right.

| Screen | File |
|---|---|
| Chats | [chat_80x24.md](chat_80x24.md) |
| Composer with the @mention popup | [composer_80x24.md](composer_80x24.md) |
| Workflows | [workflows_80x24.md](workflows_80x24.md) |
| A run waiting on its gate | [workflow_run_80x24.md](workflow_run_80x24.md) |
| Proposals | [proposals_80x24.md](proposals_80x24.md) |
| Proposals with the diff | [diff_80x24.md](diff_80x24.md) |
| Agents (panes by priority) | [agents_80x24.md](agents_80x24.md) |
| Hosts | [hosts_80x24.md](hosts_80x24.md) |
| Task state and memory | [memory_80x24.md](memory_80x24.md) |
| People | [people_80x24.md](people_80x24.md) |
| Notifications | [notifications_80x24.md](notifications_80x24.md) |
| Settings | [settings_80x24.md](settings_80x24.md) |
| Help (scrollable) | [help_80x24.md](help_80x24.md) |
