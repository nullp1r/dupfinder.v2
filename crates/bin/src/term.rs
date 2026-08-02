use std::fmt::Write as _;
use std::time::{Duration, Instant};
use std::{fmt, io, io::prelude::*};

const TICK: Duration = Duration::from_nanos(1_000_000_000 / 60);

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

pub struct ProgressBar<'a, W: Write> {
  w: &'a mut W,
  s: String,
  t: Instant,
  cur: usize,
  max: usize,
}

impl<W: Write> Drop for ProgressBar<'_, W> {
  fn drop(&mut self) {
    let _ = self.render();
    let _ = writeln!(self.w, "{CURSOR_SHOW}");
  }
}

impl<'a, W: Write> ProgressBar<'a, W> {
  pub fn new(w: &'a mut W, max: usize) -> io::Result<Self> {
    write!(w, "{CURSOR_HIDE}")?;
    let s = Default::default();
    let t = Instant::now() - TICK;
    Ok(Self { w, s, t, cur: 0, max })
  }

  pub fn update(&mut self, n: usize) -> io::Result<()> {
    self.cur = n;
    let now = Instant::now();
    if now.duration_since(self.t) >= TICK {
      self.t = now;
      self.render()?;
    }
    Ok(())
  }

  fn render(&mut self) -> io::Result<()> {
    const N: usize = cfg_select! { windows => 4, _ => 8 };
    const W: usize = 50;

    let max = self.max.max(1);
    let cur = self.cur.min(max);

    let n = N * W * cur / max;
    let [filled, rem] = [n / N, n % N];
    let empty = W - filled;

    // windows => U+2591..=U+2593, _ => U+2589..=U+258F
    let utf8 = [0xe2, 0x96, cfg_select! { windows => 0x90 + rem, _ => 0x8f - rem } as u8];
    let mid = unsafe { str::from_utf8_unchecked(&utf8) };
    let mid = if let 0 = rem { "" } else { mid };

    let color = if cur == max { 2 } else { 3 };
    let pad = max.ilog10() as usize + 1;
    let pct = 100 * cur / max;

    let bar = format_args!("\x1b[9{color};40m{:█>filled$}{mid:<empty$}\x1b[49m {pct:3}%\x1b[39m", "");
    let count = format_args!("\x1b[2m({:pad$} / {})\x1b[22m", self.cur, self.max);

    self.s.clear();
    let _ = write!(self.s, "\r{bar} {count}");

    self.w.write_all(self.s.as_bytes())?;
    self.w.flush()
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
