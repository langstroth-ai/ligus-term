//! Keybindings: actions, the contexts they live in, defaults, and overrides
//! from the config file.
//!
//! A key is looked up in the contexts that are active for the focused part
//! of the screen, most specific first (for example the proposal list uses
//! `review`, then `nav`, then `global`). While the composer has focus only
//! `composer` applies, plus `global` chords that carry Ctrl or Alt, so
//! letters always type.
//!
//! Override in `config.toml`, per context, by action name; an override
//! replaces that action's default keys in that context:
//!
//! ```toml
//! [keys.global]
//! help = ["?", "f1"]
//! [keys.composer]
//! newline = ["alt-enter", "ctrl-j"]
//! ```

use std::collections::HashMap;
use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Context {
    Global,
    Nav,
    Chat,
    Composer,
    Review,
    Diff,
    People,
    Agents,
    Settings,
    Notifications,
    Workflows,
    Hosts,
}

impl Context {
    pub const ALL: [Context; 12] = [
        Context::Global,
        Context::Nav,
        Context::Chat,
        Context::Composer,
        Context::Review,
        Context::Diff,
        Context::People,
        Context::Agents,
        Context::Settings,
        Context::Notifications,
        Context::Workflows,
        Context::Hosts,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Context::Global => "global",
            Context::Nav => "nav",
            Context::Chat => "chat",
            Context::Composer => "composer",
            Context::Review => "review",
            Context::Diff => "diff",
            Context::People => "people",
            Context::Agents => "agents",
            Context::Settings => "settings",
            Context::Notifications => "notifications",
            Context::Workflows => "workflows",
            Context::Hosts => "hosts",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Context::Global => "Anywhere",
            Context::Nav => "Lists",
            Context::Chat => "Chats",
            Context::Composer => "Composer",
            Context::Review => "Proposals",
            Context::Diff => "Diff viewer",
            Context::People => "People",
            Context::Agents => "Agents",
            Context::Settings => "Settings",
            Context::Notifications => "Notifications",
            Context::Workflows => "Workflows",
            Context::Hosts => "Hosts",
        }
    }
    pub fn from_name(s: &str) -> Option<Context> {
        Context::ALL.into_iter().find(|c| c.name() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
    Help,
    Palette,
    SwitchTheme,
    SwitchAppearance,
    WhichKey,
    Messages,
    Back,
    SwitchWorkspace,
    AddWorkspace,
    LinkDevice,
    GoChats,
    GoProposals,
    GoAgents,
    GoHosts,
    GoPeople,
    GoNotifications,
    GoSettings,
    GoWorkflows,
    GoMemory,
    FocusNext,
    FocusPrev,
    ChatList,
    ViewBack,
    ViewForward,
    FocusMode,
    Find,
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Select,
    RowMenu,
    Compose,
    StopTurn,
    ToggleTools,
    DumpTranscript,
    NewChat,
    LockChat,
    ClearLockdown,
    Send,
    Newline,
    CancelCompose,
    Complete,
    EditInEditor,
    SearchHistory,
    Approve,
    Reject,
    ApplyOnHost,
    WithdrawApply,
    NextHunk,
    PrevHunk,
    NextFile,
    PrevFile,
    ChangeRole,
    RemoveMember,
    Invite,
    ChangeRuntime,
    ToggleAccess,
    AddAgent,
    Toggle,
    MarkAllRead,
    StartRun,
    ApproveGate,
    RejectGate,
    CancelRun,
    BindCheckout,
    AdmitBox,
    Setup,
    CopySafetyCode,
}

/// One row of the binding table: the single source for defaults, config
/// names and the help overlay.
pub struct Binding {
    pub context: Context,
    pub action: Action,
    pub name: &'static str,
    pub help: &'static str,
    pub keys: &'static [&'static str],
}

macro_rules! b {
    ($ctx:ident, $act:ident, $name:literal, $help:literal, [$($k:literal),*]) => {
        Binding { context: Context::$ctx, action: Action::$act, name: $name, help: $help, keys: &[$($k),*] }
    };
}

pub const BINDINGS: &[Binding] = &[
    b!(Global, Quit, "quit", "Quit (asks first while work runs here)", ["ctrl-c", "q"]),
    b!(Global, Help, "help", "Show or hide this help", ["?", "f1"]),
    b!(Global, Palette, "palette", "Command palette: every action, view, chat, proposal, run and agent", [":", "ctrl-p", "ctrl-k"]),
    b!(Global, SwitchTheme, "switch_theme", "Switch the theme (this session)", []),
    b!(Global, SwitchAppearance, "switch_appearance", "Switch light or dark (this session)", []),
    b!(Global, WhichKey, "which_key", "Show the keys you can press here (any key then acts)", ["space"]),
    b!(Global, Messages, "messages", "This session's notices and errors, in full", ["!"]),
    b!(Global, Back, "back", "Back (to the view you came from) / close", ["esc"]),
    b!(Global, SwitchWorkspace, "switch_workspace", "Switch workspace", ["w", "ctrl-w"]),
    b!(Global, AddWorkspace, "add_workspace", "Create or join a workspace", ["+"]),
    b!(Global, Setup, "setup", "Set up: an agent, a tool, a folder, a box, a phone, an invite, a relay", ["u"]),
    b!(Global, LinkDevice, "link_device", "Link this computer to my devices", []),
    b!(Global, GoChats, "go_chats", "Chats", ["c", "1"]),
    b!(Global, GoWorkflows, "go_workflows", "Workflows and runs", ["f", "2"]),
    b!(Global, GoProposals, "go_proposals", "Proposals", ["p", "3"]),
    b!(Global, GoAgents, "go_agents", "Agents and hand-offs", ["a", "4"]),
    b!(Global, GoHosts, "go_hosts", "Hosts and workers", ["h", "5"]),
    b!(Global, GoMemory, "go_memory", "Task state and memory", ["y", "6"]),
    b!(Global, GoPeople, "go_people", "People", ["m", "7"]),
    b!(Global, GoNotifications, "go_notifications", "Notifications", ["n", "8"]),
    b!(Global, GoSettings, "go_settings", "Settings", ["s", "9"]),
    b!(Global, FocusNext, "focus_next", "Next pane", ["tab"]),
    b!(Global, FocusPrev, "focus_prev", "Previous pane", ["backtab"]),
    b!(Global, StopTurn, "stop_turn", "Stop the agent's turn in this chat (when it runs on this device)", ["ctrl-x"]),
    b!(Global, ViewBack, "view_back", "The view you were in before", ["ctrl-o", "alt-left"]),
    b!(Global, ViewForward, "view_forward", "Forward again after going back", ["alt-right"]),
    b!(Global, FocusMode, "focus_mode", "Focus mode: hide the top bar and the key hints (again to bring them back)", ["ctrl-f"]),
    b!(Global, ChatList, "chat_list", "The chat list (over the chat in a narrow terminal)", ["ctrl-l"]),
    b!(Nav, Up, "up", "Up", ["up", "k"]),
    b!(Nav, Down, "down", "Down", ["down", "j"]),
    b!(Nav, PageUp, "page_up", "Page up", ["pageup", "ctrl-u"]),
    b!(Nav, PageDown, "page_down", "Page down", ["pagedown", "ctrl-d"]),
    b!(Nav, Top, "top", "First", ["home", "g"]),
    b!(Nav, Bottom, "bottom", "Last", ["end", "G"]),
    b!(Nav, Select, "select", "Open (the row's main action)", ["enter", "l", "right"]),
    b!(Nav, RowMenu, "row_menu", "Actions for this row and this view (destructive ones last, and they ask first)", [".", "shift-f10"]),
    b!(Nav, Find, "find", "Find in this list (as you type; ↑↓ next match)", ["/"]),
    b!(Chat, Compose, "compose", "Write a message", ["i"]),
    b!(Chat, NewChat, "new_chat", "New chat", ["N"]),
    b!(Chat, DumpTranscript, "dump_transcript", "The whole transcript in $PAGER (or your terminal's scrollback)", ["D"]),
    b!(Chat, ToggleTools, "toggle_tools", "Show or hide what tool calls returned", ["o"]),
    b!(Chat, LockChat, "lock_chat", "Lock this chat down", ["L"]),
    b!(Chat, ClearLockdown, "clear_lockdown", "Clear the lockdown", ["U"]),
    b!(Composer, Send, "send", "Send", ["enter"]),
    b!(Composer, Newline, "newline", "New line", ["alt-enter", "ctrl-j", "shift-enter"]),
    b!(Composer, CancelCompose, "cancel", "Leave the composer (keeps the draft)", ["esc"]),
    b!(Composer, Complete, "complete", "Accept the @mention", ["tab"]),
    b!(Composer, EditInEditor, "edit_in_editor", "Write the draft in $VISUAL or $EDITOR", ["ctrl-g"]),
    b!(Composer, SearchHistory, "search_history", "Search what you sent (↑ on an empty draft recalls it)", ["ctrl-r"]),
    b!(Review, Approve, "approve", "Approve", ["A"]),
    b!(Review, Reject, "reject", "Reject (asks first)", ["R"]),
    b!(Review, ApplyOnHost, "apply_on_host", "Apply on a host", ["O"]),
    b!(Review, WithdrawApply, "withdraw_apply", "Withdraw the apply request", ["W"]),
    b!(Diff, NextHunk, "next_hunk", "Next hunk", ["]"]),
    b!(Diff, PrevHunk, "prev_hunk", "Previous hunk", ["["]),
    b!(Diff, NextFile, "next_file", "Next file", ["}"]),
    b!(Diff, PrevFile, "prev_file", "Previous file", ["{"]),
    b!(People, ChangeRole, "change_role", "Change role", ["r"]),
    b!(People, RemoveMember, "remove_member", "Remove from the workspace", ["X"]),
    b!(People, Invite, "invite", "Create an invite", ["I"]),
    b!(People, AdmitBox, "admit_box", "Admit a box (ligus serve or worker)", ["B"]),
    b!(People, LinkDevice, "link_device", "Link to my devices: this terminal as one of your devices, not its own member", ["D"]),
    b!(Agents, ChangeRuntime, "change_runtime", "Change the agent's runtime", ["r"]),
    b!(Agents, ToggleAccess, "toggle_access", "Who can use the agent: only me or everyone", ["E"]),
    b!(Agents, AddAgent, "add_agent", "Add an agent on a runtime this device has", ["N"]),
    b!(Settings, Toggle, "toggle", "Toggle", ["enter", "t"]),
    b!(Settings, LinkDevice, "link_device", "Link to my devices: this terminal as one of your devices, not its own member", ["D"]),
    b!(Settings, CopySafetyCode, "copy_safety_code", "Copy your safety code", []),
    b!(Notifications, MarkAllRead, "mark_all_read", "Mark all read", ["M"]),
    b!(Workflows, StartRun, "start_run", "Start a run of this workflow", ["S"]),
    b!(Workflows, ApproveGate, "approve_gate", "Approve the run's waiting gate", ["A"]),
    b!(Workflows, RejectGate, "reject_gate", "Reject the run's waiting gate (asks first)", ["R"]),
    b!(Workflows, CancelRun, "cancel_run", "Cancel the run (asks first)", ["X"]),
    b!(Hosts, BindCheckout, "bind_checkout", "Set this device's checkout for apply", ["b"]),
];

/// A key with modifiers, as written in config (`ctrl-c`, `alt-enter`, `G`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Chord {
    /// Normalize a terminal key event: for printable characters the case
    /// already says whether Shift was held, so Shift is dropped.
    pub fn from_event(e: &KeyEvent) -> Chord {
        let mut mods = e.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let code = match e.code {
            KeyCode::Char(c) => {
                mods.remove(KeyModifiers::SHIFT);
                // Ctrl-letters arrive lowercase on most terminals.
                if mods.contains(KeyModifiers::CONTROL) { KeyCode::Char(c.to_ascii_lowercase()) } else { KeyCode::Char(c) }
            }
            KeyCode::BackTab => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            other => other,
        };
        Chord { code, mods }
    }

    pub fn parse(s: &str) -> Result<Chord, String> {
        let mut mods = KeyModifiers::NONE;
        let mut rest = s;
        loop {
            let lower = rest.to_ascii_lowercase();
            let (m, n) = if lower.starts_with("ctrl-") {
                (KeyModifiers::CONTROL, 5)
            } else if lower.starts_with("alt-") {
                (KeyModifiers::ALT, 4)
            } else if lower.starts_with("shift-") && rest.len() > 6 {
                (KeyModifiers::SHIFT, 6)
            } else {
                break;
            };
            mods |= m;
            rest = &rest[n..];
        }
        let code = match rest.to_ascii_lowercase().as_str() {
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "space" => KeyCode::Char(' '),
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => KeyCode::F(f[1..].parse().unwrap()),
            _ => {
                let mut chars = rest.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => {
                        let c = if mods.contains(KeyModifiers::CONTROL) { c.to_ascii_lowercase() } else { c };
                        if mods.contains(KeyModifiers::SHIFT) && c.is_alphabetic() {
                            mods.remove(KeyModifiers::SHIFT);
                            return Ok(Chord { code: KeyCode::Char(c.to_ascii_uppercase()), mods });
                        }
                        KeyCode::Char(c)
                    }
                    _ => return Err(format!("unknown key “{s}”")),
                }
            }
        };
        if matches!(code, KeyCode::BackTab) {
            mods.remove(KeyModifiers::SHIFT);
        }
        Ok(Chord { code, mods })
    }

    /// Whether this chord can't be typed text (so it may act while typing).
    pub fn is_command(&self) -> bool {
        self.mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) || matches!(self.code, KeyCode::F(_))
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.contains(KeyModifiers::CONTROL) {
            f.write_str("ctrl-")?;
        }
        if self.mods.contains(KeyModifiers::ALT) {
            f.write_str("alt-")?;
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            f.write_str("shift-")?;
        }
        match self.code {
            KeyCode::Enter => f.write_str("enter"),
            KeyCode::Esc => f.write_str("esc"),
            KeyCode::Tab => f.write_str("tab"),
            KeyCode::BackTab => f.write_str("shift-tab"),
            KeyCode::Char(' ') => f.write_str("space"),
            KeyCode::Backspace => f.write_str("backspace"),
            KeyCode::Delete => f.write_str("delete"),
            KeyCode::Up => f.write_str("↑"),
            KeyCode::Down => f.write_str("↓"),
            KeyCode::Left => f.write_str("←"),
            KeyCode::Right => f.write_str("→"),
            KeyCode::Home => f.write_str("home"),
            KeyCode::End => f.write_str("end"),
            KeyCode::PageUp => f.write_str("pgup"),
            KeyCode::PageDown => f.write_str("pgdn"),
            KeyCode::F(n) => write!(f, "f{n}"),
            KeyCode::Char(c) => write!(f, "{c}"),
            _ => f.write_str("?"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Keymap {
    map: HashMap<(Context, Chord), Action>,
    by_action: HashMap<(Context, Action), Vec<Chord>>,
}

/// Overrides as read from config: context name → action name → keys.
pub type Overrides = HashMap<String, HashMap<String, Vec<String>>>;

impl Default for Keymap {
    fn default() -> Self {
        Keymap::with_overrides(&Overrides::new()).expect("default keymap is valid")
    }
}

impl Keymap {
    pub fn with_overrides(over: &Overrides) -> Result<Keymap, String> {
        let mut by_action: HashMap<(Context, Action), Vec<Chord>> = HashMap::new();
        for b in BINDINGS {
            let keys = b.keys.iter().map(|k| Chord::parse(k)).collect::<Result<Vec<_>, _>>()?;
            by_action.insert((b.context, b.action), keys);
        }
        for (ctx_name, actions) in over {
            let ctx = Context::from_name(ctx_name).ok_or_else(|| format!("[keys.{ctx_name}]: no such context"))?;
            for (name, keys) in actions {
                let b = BINDINGS
                    .iter()
                    .find(|b| b.context == ctx && b.name == name)
                    .ok_or_else(|| format!("[keys.{ctx_name}] {name}: no such action in {ctx_name}"))?;
                let chords = keys
                    .iter()
                    .map(|k| Chord::parse(k).map_err(|e| format!("[keys.{ctx_name}] {name}: {e}")))
                    .collect::<Result<Vec<_>, _>>()?;
                by_action.insert((ctx, b.action), chords);
            }
        }
        let mut map = HashMap::new();
        // Insert in table order so a clash is reported against a stable pair.
        for b in BINDINGS {
            for chord in &by_action[&(b.context, b.action)] {
                if let Some(prev) = map.insert((b.context, *chord), b.action) {
                    let prev_name = BINDINGS.iter().find(|x| x.context == b.context && x.action == prev).map(|x| x.name).unwrap_or("?");
                    return Err(format!("[keys.{}] “{chord}” is bound to both {prev_name} and {}", b.context.name(), b.name));
                }
            }
        }
        Ok(Keymap { map, by_action })
    }

    /// The action a key means, in the given contexts (most specific first).
    /// `typing` is set while a text field has focus: then only command
    /// chords reach `global`.
    pub fn resolve(&self, contexts: &[Context], key: &KeyEvent, typing: bool) -> Option<Action> {
        let chord = Chord::from_event(key);
        for &ctx in contexts {
            if typing && ctx == Context::Global && !chord.is_command() {
                continue;
            }
            if let Some(a) = self.map.get(&(ctx, chord)) {
                return Some(*a);
            }
        }
        None
    }

    /// The keys for an action, for hints ("A approve").
    pub fn keys(&self, ctx: Context, action: Action) -> &[Chord] {
        self.by_action.get(&(ctx, action)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// The first key for an action, as text.
    pub fn hint(&self, ctx: Context, action: Action) -> String {
        self.keys(ctx, action).first().map(|c| c.to_string()).unwrap_or_else(|| "unbound".into())
    }
}
