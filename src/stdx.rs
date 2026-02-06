pub mod fmt {
  use std::fmt;

  pub struct Size(pub u64); // bytes
  pub struct Time(pub u64); // nanoseconds

  impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      let [s, u] = f.width().map_or([0, 0], |w| [w, 2]);
      let i = (self.0 | 1).ilog2() as usize / 10;
      let unit = ["", "Ki", "Mi", "Gi", "Ti", "Pi", "Ei"][i];
      let size = self.0 as f64 / (1u64 << 10 * i) as f64;
      let prec = if i > 0 { precision(size) } else { 0 };
      write!(f, "{size:s$.prec$} {unit:u$}B")
    }
  }

  impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      let [t, u] = f.width().map_or([0, 0], |w| [w, 1]);
      let i = self.0.clamp(1, 1_000_000_000).ilog10() as usize / 3;
      let unit = ["n", "µ", "m", ""][i];
      let time = self.0 as f64 / [1e0, 1e3, 1e6, 1e9][i];
      let prec = if i > 0 { precision(time) } else { 0 };
      write!(f, "{time:t$.prec$} {unit:u$}s")
    }
  }

  fn precision(value: f64) -> usize {
    match value {
      99.95.. => 0,
      9.995.. => 1,
      _ => 2,
    }
  }
}

pub mod hash {
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::{collections, hash};

  pub type HashMap<K, V> = collections::HashMap<K, V, Builder>;

  pub struct Builder {
    seed: u64,
  }

  impl Default for Builder {
    fn default() -> Self {
      static SEED: AtomicU64 = AtomicU64::new(!0);

      Self { seed: SEED.fetch_sub(1, Ordering::Relaxed) }
    }
  }

  impl hash::BuildHasher for Builder {
    type Hasher = Hasher;

    fn build_hasher(&self) -> Self::Hasher {
      Hasher { state: self.seed }
    }
  }

  pub struct Hasher {
    state: u64,
  }

  impl Default for Hasher {
    fn default() -> Self {
      Self { state: !0 }
    }
  }

  impl hash::Hasher for Hasher {
    fn write(&mut self, input: &[u8]) {
      let (chunks, tail) = input.as_chunks();
      for chunk in chunks {
        self.write_u64(u64::from_ne_bytes(*chunk));
      }
      if let n @ 1.. = tail.len() {
        self.write_u64(tail.iter().rev().fold(n as u64, |acc, &b| acc << 8 | b as u64));
      }
    }

    fn write_u64(&mut self, input: u64) {
      self.state = (self.state ^ input).wrapping_mul(0x9e3779b97f4a7c15);
    }

    fn finish(&self) -> u64 {
      let mut x = self.state;
      x = (x ^ x >> 27).wrapping_mul(0x3c79ac492ba7b653);
      x = (x ^ x >> 33).wrapping_mul(0x1c69b3f74ac4ae35);
      x ^ x >> 27
    }
  }
}

pub mod fs {
  use std::fs::{DirEntry, File};
  use std::hash::Hasher as _;
  use std::io::SeekFrom;
  use std::path::{Path, PathBuf};
  use std::{fs, io, io::prelude::*};

  use super::hash::Hasher;

  #[derive(Clone, Copy)]
  pub enum FileHash {
    Full,
    Prefix(u64),
    Suffix(u64),
  }

  impl FileHash {
    pub fn compute(self, path: &Path) -> io::Result<u64> {
      let mut file = File::open(path)?;
      let mut file = match self {
        Self::Full => file.take(u64::MAX),
        Self::Prefix(max_bytes) => file.take(max_bytes),
        Self::Suffix(max_bytes) => {
          let size = file.seek(SeekFrom::End(0))?;
          file.seek(SeekFrom::Start(size.saturating_sub(max_bytes)))?;
          file.take(u64::MAX)
        }
      };

      let mut hasher = Hasher::default();
      let mut buffer = [0; 1 << 17]; // 128 KiB
      let buffer = self.limited(&mut buffer);
      loop {
        match file.read(buffer)? {
          0 => break Ok(hasher.finish()),
          n => hasher.write(&buffer[..n]),
        }
      }
    }

    fn limited(self, buf: &mut [u8]) -> &mut [u8] {
      if let Self::Prefix(limit) | Self::Suffix(limit) = self
        && limit < buf.len() as u64
      {
        &mut buf[..limit as usize]
      } else {
        &mut buf[..]
      }
    }
  }

  pub fn scan<F>(cwd: &Path, f: &mut F) -> io::Result<()>
  where
    F: FnMut(&Path, io::Result<(PathBuf, DirEntry)>) -> io::Result<()>,
  {
    let entries = match fs::read_dir(cwd) {
      Err(err) => return f(cwd, Err(err)),
      Ok(entries) => entries,
    };

    for e in entries {
      match e.and_then(|e| Ok((e.file_type()?, e.path(), e))) {
        Err(err) => f(cwd, Err(err))?,
        Ok((file_type, path, entry)) => match file_type {
          ft if ft.is_dir() => scan(&path, f)?,
          ft if ft.is_file() => f(cwd, Ok((path, entry)))?,
          _ => {}
        },
      }
    }

    Ok(())
  }
}

pub mod ansi {
  pub mod progress {
    use std::time::{Duration, Instant};
    use std::{fmt, io, io::prelude::*};

    const TICK: Duration = Duration::new(0, 1_000_000_000 / 50);

    const TAIL_CLEAR: &str = "\x1b[K";

    const CURSOR_HIDE: &str = "\x1b[?25l";
    const CURSOR_SHOW: &str = "\x1b[?25h";
    const CURSOR_SAVE: &str = "\x1b[s";
    const CURSOR_LOAD: &str = "\x1b[u";

    pub struct Progress<W: Write> {
      w: W,
      t: Instant,
    }

    impl<W: Write> Drop for Progress<W> {
      fn drop(&mut self) {
        let _ = writeln!(self.w, "{CURSOR_SHOW}");
      }
    }

    impl<W: Write> Progress<W> {
      pub fn new(mut w: W, fmt: fmt::Arguments<'_>) -> io::Result<Self> {
        write!(w, "{CURSOR_HIDE}{fmt}: {CURSOR_SAVE}")?;
        w.flush()?;

        Ok(Self { w, t: Instant::now() - TICK })
      }

      pub fn update(&mut self, fmt: fmt::Arguments<'_>) -> io::Result<()> {
        write!(self.w, "{CURSOR_LOAD}{fmt}{TAIL_CLEAR}")?;

        let now = Instant::now();
        if now.duration_since(self.t) >= TICK {
          self.w.flush()?;
          self.t = now;
        }

        Ok(())
      }
    }
  }

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
