//! A small text buffer for single-line fields and the multi-line composer.
//! The cursor is a byte index that always sits on a char boundary.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextBuf {
    pub text: String,
    pub cursor: usize,
    pub multiline: bool,
}

impl TextBuf {
    pub fn single() -> Self {
        Self::default()
    }
    pub fn multi() -> Self {
        Self { multiline: true, ..Self::default() }
    }
    pub fn with(text: &str) -> Self {
        Self { text: text.into(), cursor: text.len(), multiline: false }
    }
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
    pub fn set(&mut self, text: &str) {
        self.text = text.into();
        self.cursor = self.text.len();
    }

    pub fn insert_str(&mut self, s: &str) {
        let s: String = if self.multiline { s.replace("\r\n", "\n").replace('\r', "\n") } else { s.replace(['\r', '\n'], " ") };
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
    }

    pub fn insert(&mut self, c: char) {
        if c == '\n' && !self.multiline {
            return;
        }
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn prev_boundary(&self) -> usize {
        self.text[..self.cursor].char_indices().next_back().map(|(i, _)| i).unwrap_or(0)
    }
    fn next_boundary(&self) -> usize {
        self.text[self.cursor..].chars().next().map(|c| self.cursor + c.len_utf8()).unwrap_or(self.cursor)
    }
    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }
    fn line_end(&self) -> usize {
        self.text[self.cursor..].find('\n').map(|i| self.cursor + i).unwrap_or(self.text.len())
    }

    /// The column (in chars) on the cursor's line.
    fn column(&self) -> usize {
        self.text[self.line_start()..self.cursor].chars().count()
    }

    fn to_column(&self, start: usize, col: usize) -> usize {
        let end = self.text[start..].find('\n').map(|i| start + i).unwrap_or(self.text.len());
        self.text[start..end].char_indices().nth(col).map(|(i, _)| start + i).unwrap_or(end)
    }

    /// Handle an editing key; returns whether it was one.
    pub fn handle(&mut self, key: &KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('a') if ctrl => self.cursor = self.line_start(),
            KeyCode::Char('e') if ctrl => self.cursor = self.line_end(),
            KeyCode::Char('u') if ctrl => {
                let s = self.line_start();
                self.text.replace_range(s..self.cursor, "");
                self.cursor = s;
            }
            KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => self.insert(c),
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => self.delete_word(),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let p = self.prev_boundary();
                    self.text.replace_range(p..self.cursor, "");
                    self.cursor = p;
                }
            }
            KeyCode::Delete => {
                let n = self.next_boundary();
                self.text.replace_range(self.cursor..n, "");
            }
            KeyCode::Left => self.cursor = self.prev_boundary(),
            KeyCode::Right => self.cursor = self.next_boundary(),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => self.cursor = self.line_end(),
            KeyCode::Up if self.multiline => {
                let s = self.line_start();
                if s == 0 {
                    return false;
                }
                let col = self.column();
                let prev = self.text[..s - 1].rfind('\n').map(|i| i + 1).unwrap_or(0);
                self.cursor = self.to_column(prev, col);
            }
            KeyCode::Down if self.multiline => {
                let e = self.line_end();
                if e == self.text.len() {
                    return false;
                }
                let col = self.column();
                self.cursor = self.to_column(e + 1, col);
            }
            _ => return false,
        }
        true
    }

    fn delete_word(&mut self) {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end_matches(' ');
        let start = trimmed.rfind([' ', '\n']).map(|i| i + 1).unwrap_or(0);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// The `@word` being typed at the cursor: (start byte of `@`, query).
    pub fn mention_query(&self) -> Option<(usize, String)> {
        let before = &self.text[..self.cursor];
        let start = before.rfind(|c: char| c.is_whitespace()).map(|i| i + 1).unwrap_or(0);
        let word = &before[start..];
        let q = word.strip_prefix('@')?;
        if q.contains('@') {
            return None;
        }
        Some((start, q.to_string()))
    }

    /// Replace the `@query` at the cursor with `@handle `.
    pub fn complete_mention(&mut self, handle: &str) {
        if let Some((start, _)) = self.mention_query() {
            let ins = format!("@{handle} ");
            self.text.replace_range(start..self.cursor, &ins);
            self.cursor = start + ins.len();
        }
    }

    /// (row, display column) of the cursor, for placing the terminal cursor.
    pub fn cursor_pos(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        let row = before.matches('\n').count();
        let line = &before[self.line_start()..];
        (row, line.width())
    }

    pub fn lines(&self) -> Vec<&str> {
        self.text.split('\n').collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn edits_and_moves() {
        let mut t = TextBuf::multi();
        for c in "héllo".chars() {
            t.handle(&key(KeyCode::Char(c)));
        }
        t.insert('\n');
        t.insert_str("wörld");
        assert_eq!(t.cursor_pos(), (1, 5));
        t.handle(&key(KeyCode::Up));
        assert_eq!(t.cursor_pos(), (0, 5));
        t.handle(&key(KeyCode::Left));
        t.handle(&key(KeyCode::Backspace));
        assert_eq!(t.text, "hélo\nwörld");
        t.handle(&key(KeyCode::Down));
        assert_eq!(t.cursor_pos(), (1, 3));
    }

    #[test]
    fn single_line_flattens_paste() {
        let mut t = TextBuf::single();
        t.insert_str("a\nb\r\nc");
        assert_eq!(t.text, "a b  c");
    }

    #[test]
    fn mention_query_and_complete() {
        let mut t = TextBuf::multi();
        t.insert_str("hey @co");
        assert_eq!(t.mention_query(), Some((4, "co".into())));
        t.complete_mention("coder");
        assert_eq!(t.text, "hey @coder ");
        assert_eq!(t.mention_query(), None);
        t.insert_str("mail@x");
        assert_eq!(t.mention_query(), None);
    }
}
