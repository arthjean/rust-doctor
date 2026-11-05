use std::fmt::Write as _;
use std::io;

use console::{Key, Term};

use super::text::Line;

pub struct Canvas {
    term: Term,
    color: bool,
    links: bool,
    drawn: usize,
}

impl Canvas {
    pub const fn new(term: Term, color: bool) -> Self {
        Self {
            term,
            color,
            links: color,
            drawn: 0,
        }
    }

    pub fn is_attended(&self) -> bool {
        self.term.features().is_attended()
    }

    pub fn geometry(&self) -> (usize, usize) {
        let (rows, columns) = self.term.size();
        (usize::from(columns).max(1), usize::from(rows).max(1))
    }

    pub fn hide_cursor(&self) -> io::Result<()> {
        self.term.hide_cursor()
    }

    pub fn read_key(&self) -> io::Result<Key> {
        self.term.read_key_raw()
    }

    pub fn restore(&self) {
        let _ = self.term.show_cursor();
        let _ = self.term.flush();
    }

    pub fn draw(&mut self, frame: Vec<Line>, columns: usize, rows: usize) -> io::Result<()> {
        let height = rows.saturating_sub(1).max(1);
        let width = columns.saturating_sub(1).max(1);
        let mut buffer = String::new();
        if self.drawn > 0 {
            let _ = write!(buffer, "\u{1b}[{}A", self.drawn);
        }
        buffer.push('\r');
        let mut written = 0usize;
        for line in frame.into_iter().take(height) {
            buffer.push_str(&line.truncate_end(width).render(self.color, self.links));
            buffer.push_str("\u{1b}[K\r\n");
            written = written.saturating_add(1);
        }
        buffer.push_str("\u{1b}[J");
        self.term.write_str(&buffer)?;
        self.term.flush()?;
        self.drawn = written;
        Ok(())
    }
}
