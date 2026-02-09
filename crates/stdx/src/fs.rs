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
