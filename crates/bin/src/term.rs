use std::fmt::Write as _;
use std::time::{Duration, Instant};
use std::{fmt, io, io::prelude::*};

const TICK: Duration = Duration::from_millis(1000 / 50);

const TAIL_CLEAR: &str = "\x1b[K";
const CURSOR_HIDE: &str = "\x1b[?25l";
const CURSOR_SHOW: &str = "\x1b[?25h";
const CURSOR_SAVE: &str = "\x1b[s";
const CURSOR_LOAD: &str = "\x1b[u";

pub struct Progress<W: Write> {
  w: W,
  s: String,
  t: Instant,
}

impl<W: Write> Drop for Progress<W> {
  fn drop(&mut self) {
    let _ = writeln!(self.w, "{CURSOR_LOAD}{}{TAIL_CLEAR}{CURSOR_SHOW}", self.s);
  }
}

impl<W: Write> Progress<W> {
  pub fn new(mut w: W, fmt: fmt::Arguments<'_>) -> io::Result<Self> {
    write!(w, "{CURSOR_HIDE}{fmt}: {CURSOR_SAVE}")?;
    w.flush()?;

    let s = Default::default();
    let t = Instant::now() - TICK;
    Ok(Self { w, s, t })
  }

  pub fn update(&mut self, fmt: fmt::Arguments<'_>) -> io::Result<()> {
    self.s.clear();
    let _ = self.s.write_fmt(fmt);

    let now = Instant::now();
    if now.duration_since(self.t) >= TICK {
      write!(self.w, "{CURSOR_LOAD}{}{TAIL_CLEAR}", self.s)?;
      self.w.flush()?;
      self.t = now;
    }

    Ok(())
  }
}

pub mod ansi {
  #[cfg(not(windows))]
  pub fn enable() {}

  #[cfg(windows)]
  pub fn enable() {
    use windows_sys::Win32::{Foundation::*, System::Console::*};

    for id in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
      unsafe {
        let h = GetStdHandle(id);
        if h.is_null() || h == INVALID_HANDLE_VALUE {
          continue;
        }

        let mut mode = 0;
        if GetConsoleMode(h, &mut mode) == FALSE {
          continue;
        }

        let mode = mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
        if SetConsoleMode(h, mode) == FALSE {
          continue;
        }
      }
    }
  }
}
