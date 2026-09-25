//! A minimal VT100/xterm screen model for terminal snapshots.
//!
//! `claude logs <short>` prints the background session's current screen as raw
//! TUI output: cursor addressing, erase sequences, SGR colours, save/restore —
//! and almost no line feeds (a verified 5 KB snapshot had 330 ESC, 51 CR and 0
//! LF). Shown as text that is garbage, so the server replays it onto a fixed
//! grid and returns plain lines. No ESC byte ever leaves this module.
//!
//! Deliberately small (no new dependency): printable text with a wide-char
//! approximation, C0 controls, ESC 7/8/D/E/M/c, CSI cursor movement, erase,
//! insert/delete, scroll region, and string sequences (OSC/DCS/APC/PM/SOS)
//! skipped. Colours and modes are ignored.

/// The rendered screen: `rows` are the grid lines with trailing blanks
/// trimmed and trailing empty lines dropped; `text` is them joined by `\n`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Screen {
    pub text: String,
    pub rows: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    Csi,
    /// OSC / DCS / APC / PM / SOS: skip until BEL or ST (`ESC \`).
    Str,
    StrEscape,
    /// `ESC (`, `ESC )`, … : one designator char follows.
    Charset,
}

struct Term {
    cols: usize,
    rows: usize,
    grid: Vec<Vec<char>>,
    r: usize,
    c: usize,
    saved: (usize, usize),
    top: usize,
    bottom: usize,
}

/// Placeholder in the cell after a double-width character.
const WIDE_TAIL: char = '\0';

impl Term {
    fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Self {
            cols,
            rows,
            grid: vec![vec![' '; cols]; rows],
            r: 0,
            c: 0,
            saved: (0, 0),
            top: 0,
            bottom: rows - 1,
        }
    }

    fn blank_row(&self) -> Vec<char> {
        vec![' '; self.cols]
    }

    fn scroll_up(&mut self, n: usize) {
        for _ in 0..n {
            if self.top < self.bottom && self.bottom < self.rows {
                self.grid.remove(self.top);
                self.grid.insert(self.bottom, self.blank_row());
            }
        }
    }

    fn scroll_down(&mut self, n: usize) {
        for _ in 0..n {
            if self.top < self.bottom && self.bottom < self.rows {
                self.grid.remove(self.bottom);
                self.grid.insert(self.top, self.blank_row());
            }
        }
    }

    fn line_feed(&mut self) {
        if self.r == self.bottom {
            self.scroll_up(1);
        } else if self.r + 1 < self.rows {
            self.r += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.r == self.top {
            self.scroll_down(1);
        } else {
            self.r = self.r.saturating_sub(1);
        }
    }

    fn put(&mut self, ch: char) {
        let w = char_width(ch);
        if w == 0 {
            return;
        }
        if self.c + w > self.cols {
            self.c = 0;
            self.line_feed();
        }
        let row = &mut self.grid[self.r];
        row[self.c] = ch;
        if w == 2 && self.c + 1 < self.cols {
            row[self.c + 1] = WIDE_TAIL;
        }
        self.c += w;
    }

    fn erase_display(&mut self, mode: usize) {
        match mode {
            0 => {
                self.erase_line(0);
                for r in self.r + 1..self.rows {
                    self.grid[r] = self.blank_row();
                }
            }
            1 => {
                self.erase_line(1);
                for r in 0..self.r {
                    self.grid[r] = self.blank_row();
                }
            }
            _ => {
                for r in 0..self.rows {
                    self.grid[r] = self.blank_row();
                }
            }
        }
    }

    fn erase_line(&mut self, mode: usize) {
        let (from, to) = match mode {
            0 => (self.c.min(self.cols), self.cols),
            1 => (0, (self.c + 1).min(self.cols)),
            _ => (0, self.cols),
        };
        for cell in &mut self.grid[self.r][from..to] {
            *cell = ' ';
        }
    }

    fn csi(&mut self, private: bool, params: &[usize], fin: u8) {
        let p = |i: usize, d: usize| params.get(i).copied().filter(|v| *v != 0).unwrap_or(d);
        let rows = self.rows;
        let cols = self.cols;
        match fin {
            b'A' => self.r = self.r.saturating_sub(p(0, 1)),
            b'B' | b'e' => self.r = (self.r + p(0, 1)).min(rows - 1),
            b'C' | b'a' => self.c = (self.c + p(0, 1)).min(cols - 1),
            b'D' => self.c = self.c.saturating_sub(p(0, 1)),
            b'E' => {
                self.r = (self.r + p(0, 1)).min(rows - 1);
                self.c = 0;
            }
            b'F' => {
                self.r = self.r.saturating_sub(p(0, 1));
                self.c = 0;
            }
            b'G' | b'`' => self.c = (p(0, 1) - 1).min(cols - 1),
            b'd' => self.r = (p(0, 1) - 1).min(rows - 1),
            b'H' | b'f' => {
                self.r = (p(0, 1) - 1).min(rows - 1);
                self.c = (p(1, 1) - 1).min(cols - 1);
            }
            b'J' => self.erase_display(params.first().copied().unwrap_or(0)),
            b'K' => self.erase_line(params.first().copied().unwrap_or(0)),
            b'X' => {
                let n = p(0, 1);
                let end = (self.c + n).min(cols);
                for cell in &mut self.grid[self.r][self.c.min(cols)..end] {
                    *cell = ' ';
                }
            }
            b'P' => {
                let n = p(0, 1).min(cols.saturating_sub(self.c));
                let row = &mut self.grid[self.r];
                for _ in 0..n {
                    if self.c < row.len() {
                        row.remove(self.c);
                        row.push(' ');
                    }
                }
            }
            b'@' => {
                let n = p(0, 1).min(cols.saturating_sub(self.c));
                let row = &mut self.grid[self.r];
                for _ in 0..n {
                    if self.c < row.len() {
                        row.insert(self.c, ' ');
                        row.pop();
                    }
                }
            }
            b'L' if self.r >= self.top && self.r <= self.bottom => {
                for _ in 0..p(0, 1) {
                    self.grid.remove(self.bottom);
                    self.grid.insert(self.r, vec![' '; cols]);
                }
            }
            b'M' if self.r >= self.top && self.r <= self.bottom => {
                for _ in 0..p(0, 1) {
                    self.grid.remove(self.r);
                    self.grid.insert(self.bottom, vec![' '; cols]);
                }
            }
            b'S' => self.scroll_up(p(0, 1)),
            b'T' if !private => self.scroll_down(p(0, 1)),
            b'r' if !private => {
                let top = p(0, 1) - 1;
                let bottom = p(1, rows) - 1;
                if top < bottom && bottom < rows {
                    self.top = top;
                    self.bottom = bottom;
                } else {
                    self.top = 0;
                    self.bottom = rows - 1;
                }
                self.r = 0;
                self.c = 0;
            }
            b's' if !private => self.saved = (self.r, self.c),
            b'u' if !private => (self.r, self.c) = self.saved,
            // Alternate screen / clear-on-switch modes: start from a blank grid.
            b'h' | b'l' if private && params.iter().any(|m| matches!(m, 47 | 1047 | 1049)) => {
                self.erase_display(2);
                self.r = 0;
                self.c = 0;
            }
            _ => {}
        }
    }

    fn into_screen(self) -> Screen {
        let mut rows: Vec<String> = self
            .grid
            .into_iter()
            .map(|row| {
                let s: String = row.into_iter().filter(|c| *c != WIDE_TAIL).collect();
                s.trim_end().to_string()
            })
            .collect();
        while rows.last().is_some_and(String::is_empty) {
            rows.pop();
        }
        Screen {
            text: rows.join("\n"),
            rows,
        }
    }
}

/// Replay `bytes` onto a `cols`×`rows` grid.
pub fn render(bytes: &[u8], cols: usize, rows: usize) -> Screen {
    let mut t = Term::new(cols, rows);
    let text = String::from_utf8_lossy(bytes);
    let mut state = State::Ground;
    let mut params: Vec<usize> = Vec::new();
    let mut current: Option<usize> = None;
    let mut private = false;

    for ch in text.chars() {
        match state {
            State::Ground => match ch {
                '\u{1b}' => state = State::Escape,
                '\r' => t.c = 0,
                '\n' | '\u{0b}' | '\u{0c}' => t.line_feed(),
                '\u{08}' => t.c = t.c.saturating_sub(1),
                '\t' => t.c = ((t.c / 8 + 1) * 8).min(t.cols - 1),
                '\u{9b}' => {
                    state = State::Csi;
                    params.clear();
                    current = None;
                    private = false;
                }
                c if (c as u32) < 0x20 || c == '\u{7f}' || ('\u{80}'..='\u{9f}').contains(&c) => {}
                c => t.put(c),
            },
            State::Escape => {
                state = State::Ground;
                match ch {
                    '[' => {
                        state = State::Csi;
                        params.clear();
                        current = None;
                        private = false;
                    }
                    ']' | 'P' | 'X' | '^' | '_' => state = State::Str,
                    '(' | ')' | '*' | '+' | '-' | '.' | '/' | '#' | '%' => state = State::Charset,
                    '7' => t.saved = (t.r, t.c),
                    '8' => (t.r, t.c) = t.saved,
                    'D' => t.line_feed(),
                    'E' => {
                        t.c = 0;
                        t.line_feed();
                    }
                    'M' => t.reverse_index(),
                    'c' => {
                        let (cols, rows) = (t.cols, t.rows);
                        t = Term::new(cols, rows);
                    }
                    _ => {}
                }
            }
            State::Csi => match ch {
                '0'..='9' => {
                    let d = ch as usize - '0' as usize;
                    current = Some(current.unwrap_or(0).saturating_mul(10).saturating_add(d));
                }
                ';' | ':' => {
                    params.push(current.take().unwrap_or(0));
                }
                '?' | '>' | '<' | '=' => private = true,
                ' '..='/' => {}
                '@'..='~' => {
                    if let Some(v) = current.take() {
                        params.push(v);
                    }
                    t.csi(private, &params, ch as u8);
                    state = State::Ground;
                }
                '\u{1b}' => state = State::Escape,
                _ => state = State::Ground,
            },
            State::Str => match ch {
                '\u{07}' | '\u{9c}' => state = State::Ground,
                '\u{1b}' => state = State::StrEscape,
                _ => {}
            },
            State::StrEscape => {
                state = if ch == '\\' {
                    State::Ground
                } else {
                    State::Str
                }
            }
            State::Charset => state = State::Ground,
        }
    }
    t.into_screen()
}

/// Terminal column width of a character: 0 for combining marks and
/// zero-width characters, 2 for East Asian wide characters and most emoji,
/// else 1 (an approximation of `wcwidth`).
fn char_width(c: char) -> usize {
    let u = c as u32;
    match u {
        0x0300..=0x036F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x200B..=0x200F
        | 0x2028..=0x202E
        | 0x2060..=0x2064
        | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F
        | 0xFEFF => 0,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F680..=0x1F6FF
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_and_line_endings() {
        let s = render(b"hello\r\nworld", 20, 5);
        assert_eq!(s.rows, vec!["hello", "world"]);
        assert_eq!(s.text, "hello\nworld");
        // A bare LF keeps the column; CR returns.
        let s = render(b"ab\ncd", 20, 5);
        assert_eq!(s.rows, vec!["ab", "  cd"]);
    }

    #[test]
    fn cursor_addressing_and_erase() {
        // Save/restore, clear, home, absolute positioning, SGR ignored.
        let raw =
            b"\x1b7\x1b8\x1b[2J\x1b[H\x1b[1mTitle\x1b[0m\x1b[3;5HX\x1b[2;1Hline two\x1b[2;5H\x1b[K";
        let s = render(raw, 20, 5);
        assert_eq!(s.rows, vec!["Title", "line", "    X"]);
        assert!(!s.text.contains('\u{1b}'));
    }

    #[test]
    fn strings_and_wrapping_and_scroll() {
        let s = render(b"\x1b]0;window title\x07ok", 10, 3);
        assert_eq!(s.rows, vec!["ok"]);
        let s = render(b"abcdefghijkl", 5, 3);
        assert_eq!(s.rows, vec!["abcde", "fghij", "kl"]);
        let s = render(b"1\r\n2\r\n3\r\n4", 5, 3);
        assert_eq!(s.rows, vec!["2", "3", "4"]);
        // Wide characters take two cells.
        let s = render("日本x".as_bytes(), 10, 2);
        assert_eq!(s.rows, vec!["日本x"]);
        let s = render("ab\x1b[1Dz".as_bytes(), 10, 2);
        assert_eq!(s.rows, vec!["az"]);
    }

    #[test]
    fn realistic_tui_snapshot() {
        // Shaped like the verified `claude logs` output: no LF at all.
        let raw = "\x1b7\x1b8\x1b[2J\x1b[H\x1b[?25l\x1b[38;5;208m✻\x1b[39m Welcome\x1b[2;1H\x1b[2m────\x1b[22m\x1b[4;1H❯ Reply OK\x1b[5;1H⏺ OK\x1b[?25h";
        let s = render(raw.as_bytes(), 200, 50);
        assert_eq!(s.rows[0], "✻ Welcome");
        assert_eq!(s.rows[1], "────");
        assert_eq!(s.rows[3], "❯ Reply OK");
        assert_eq!(s.rows[4], "⏺ OK");
        assert_eq!(s.rows.len(), 5);
    }
}
