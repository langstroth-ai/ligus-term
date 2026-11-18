# Screens

Text captures of every `lt` screen, rendered by the snapshot tests in
`tests/screens.rs` into a ratatui `TestBackend` (110×32) against the
in-memory demo backend. Colors aren't captured here; the dark, light and
mono themes render the same text.

Regenerate after a UI change:

```sh
INSTA_UPDATE=always LT_WRITE_SCREENS=1 cargo test --test screens
```

| Screen | File |
|---|---|
| First run: create or join | [onboarding.md](onboarding.md) |
| First run: join with an invite | [onboarding_join.md](onboarding_join.md) |
| First run: a validation error | [onboarding_error.md](onboarding_error.md) |
| Creating a workspace asks for a passkey | [passkey_create.md](passkey_create.md) |
| The recovery key, shown once | [recovery_key.md](recovery_key.md) |
| A new workspace | [new_workspace.md](new_workspace.md) |
| Chats: list and transcript | [chat.md](chat.md) |
| An agent streaming its reply | [chat_streaming.md](chat_streaming.md) |
| Composer with the @mention popup | [chat_mentions.md](chat_mentions.md) |
| Multi-line composer | [chat_multiline.md](chat_multiline.md) |
| A locked-down chat | [chat_lockdown.md](chat_lockdown.md) |
| Workspace switcher | [switcher.md](switcher.md) |
| Create or join another workspace | [add_workspace.md](add_workspace.md) |
| Proposals with the diff viewer | [proposals.md](proposals.md) |
| Diff viewer: a later hunk | [proposals_hunk.md](proposals_hunk.md) |
| Apply on ‹host› | [apply_host.md](apply_host.md) |
| Agents and runtimes | [agents.md](agents.md) |
| Hosts | [hosts.md](hosts.md) |
| People: roles and actions | [people.md](people.md) |
| People: refused actions with the reason | [people_refused.md](people_refused.md) |
| Changing a role | [role_picker.md](role_picker.md) |
| Confirm a removal | [confirm_remove.md](confirm_remove.md) |
| Passkey sheet | [presence.md](presence.md) |
| Passkey sheet: recovery-words file | [presence_recovery.md](presence_recovery.md) |
| Notifications | [notifications.md](notifications.md) |
| Settings | [settings.md](settings.md) |
| Help: keybindings | [help.md](help.md) |
