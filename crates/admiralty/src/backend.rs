//! BUG-37: a terminal may draw a glyph wider than the layout counted (Nerd
//! Font icons, emoji presentation, East Asian Ambiguous), and then every cell
//! written after it on the same run lands one cell right. `GlyphSafe` wraps a
//! backend and splits each draw after such a glyph, so the next cell starts
//! with an explicit cursor move: positions never depend on the terminal's
//! width table.

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};

pub struct GlyphSafe<B: Backend>(pub B);

/// Could a terminal draw this cell wider than one column? Plain ASCII and the
/// box-drawing and block characters of the frames are safe.
pub fn risky(symbol: &str) -> bool {
    symbol.chars().any(|c| !c.is_ascii() && !('\u{2500}'..='\u{259f}').contains(&c))
}

impl<B: Backend> Backend for GlyphSafe<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut run: Vec<(u16, u16, &'a Cell)> = vec![];
        for item in content {
            let split = risky(item.2.symbol());
            run.push(item);
            if split {
                // each draw call starts with a cursor move to its first cell
                self.0.draw(std::mem::take(&mut run).into_iter())?;
            }
        }
        if !run.is_empty() {
            self.0.draw(run.into_iter())?;
        }
        Ok(())
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        self.0.append_lines(n)
    }
    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.0.hide_cursor()
    }
    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.0.show_cursor()
    }
    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.0.get_cursor_position()
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.0.set_cursor_position(position)
    }
    fn clear(&mut self) -> Result<(), Self::Error> {
        self.0.clear()
    }
    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.0.clear_region(clear_type)
    }
    fn size(&self) -> Result<Size, Self::Error> {
        self.0.size()
    }
    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.0.window_size()
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush()
    }
}
