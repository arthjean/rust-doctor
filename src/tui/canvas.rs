//! The terminal the frames are written to.
//!
//! Frames are rewritten in place the way Ink does: the cursor climbs back over
//! the previous frame, every row clears to its end, and whatever the shorter
//! frame left below is erased. No alternate screen, so the last frame survives
//! in the scrollback exactly as the reference leaves it.
//!
//! Nothing about the report is known here. What is known is the one thing every
//! screen could break: a frame that reaches the last row or the last column
//! moves the cursor away from where the next rewind expects it.

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
    /// `color` also gates the hyperlinks, deliberately: a run that asked for no
    /// color is a run whose terminal is not being trusted with OSC 8 either.
    pub const fn new(term: Term, color: bool) -> Self {
        Self {
            term,
            color,
            links: color,
            drawn: 0,
        }
    }

    /// Whether there is a reader on the other end at all.
    pub fn is_attended(&self) -> bool {
        self.term.features().is_attended()
    }

    /// Terminal geometry, as columns by rows.
    pub fn geometry(&self) -> (usize, usize) {
        let (rows, columns) = self.term.size();
        (usize::from(columns).max(1), usize::from(rows).max(1))
    }

    pub fn hide_cursor(&self) -> io::Result<()> {
        self.term.hide_cursor()
    }

    /// Blocks on one key, with the terminal in raw mode for the read. This is
    /// where the loop spends its time, and it is why Ctrl-C arrives as a key
    /// rather than as a signal.
    pub fn read_key(&self) -> io::Result<Key> {
        self.term.read_key_raw()
    }

    /// Gives the terminal back. Called whatever the loop returned, including an
    /// error, since a hidden cursor outlives the process that hid it.
    pub fn restore(&self) {
        let _ = self.term.show_cursor();
        let _ = self.term.flush();
    }

    /// Draws a frame in place of the previous one, inside the terminal minus
    /// its last row and its last column.
    ///
    /// Both bounds are enforced here rather than trusted from the screens: a
    /// frame that fills the terminal exactly scrolls it, and a row that reaches
    /// the last column wraps on some emulators. Either one moves the cursor a
    /// row away from where the next rewind expects it, and every frame after
    /// that is drawn in the wrong place. Screens still size themselves; this is
    /// the guarantee that none of them can break the loop.
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
            // Clearing to the end of the row is what lets a frame shrink
            // without leaving the previous one's tail behind.
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
