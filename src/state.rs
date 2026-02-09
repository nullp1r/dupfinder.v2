use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{io, io::prelude::*, mem, thread};

use crossbeam_channel as channel;

use crate::stdx::fmt::{Size, Time};
use crate::stdx::fs::{self, FileHash, FileHash::*};
use crate::stdx::hash::HashMap;
use crate::stdx::term::progress::Progress;

type Sig = [u64; 2]; // [bitflags and file size, file hash]
type SigCount = HashMap<Sig, u64>;
type File = (Box<Path>, Sig);
type Stats = (usize, u64, Duration); // (hashes, bytes, time taken)
type Errors = Vec<(Box<Path>, bool, io::Error)>; // `true` if full path

const PARTIAL_HASH_SIZE: u64 = 1 << 12; // 4 KiB

const HASH_PREF: u64 = 0b100 << 61;
const HASH_SUFF: u64 = 0b010 << 61;
const HASH_FULL: u64 = 0b001 << 61;
const HASH_MASK: u64 = 0b111 << 61;
const SIZE_MASK: u64 = !HASH_MASK;

pub struct State<W> {
  w: W,
  root: Box<Path>,
  errs: Errors,
  files: Vec<File>,
  sig_count: SigCount,
}

impl<W: Write> State<W> {
  pub fn new(w: W, root: impl Into<PathBuf>) -> Self {
    let root = root.into().into_boxed_path();
    let (errs, files, sig_count) = Default::default();
    Self { w, root, errs, files, sig_count }
  }

  pub fn run(mut self) -> io::Result<()> {
    let mut files = Default::default();

    self.scan(&mut files)?;
    self.filter_and_count(&mut files);

    let prefix = self.compute_hashes(&mut files, Prefix(PARTIAL_HASH_SIZE))?;
    self.filter_and_count(&mut files);

    let suffix = self.compute_hashes(&mut files, Suffix(PARTIAL_HASH_SIZE))?;
    self.filter_and_count(&mut files);

    let full = self.compute_hashes(&mut files, Full)?;
    self.count(files);

    self.show_duplicates()?;
    self.show_errors()?;
    self.show_summary([prefix, suffix, full])
  }

  fn scan(&mut self, files: &mut Vec<File>) -> io::Result<()> {
    let mut total = 0;
    let mut progress = Progress::new(&mut self.w, format_args!("files found"))?;

    fs::scan(&self.root, &mut |cwd, r| {
      let r = r.map_err(|err| (cwd.to_owned().into_boxed_path(), false, err));
      let r = r.and_then(|(path, file)| match file.metadata() {
        Err(err) => Err((path.into_boxed_path(), true, err)),
        Ok(meta) => Ok((path, meta)),
      });

      match r {
        Err(err) => self.errs.push(err),
        Ok((path, meta)) => {
          total += meta.len();
          files.push((path.into_boxed_path(), [meta.len(), 0]));
          progress.update(format_args!("\x1b[93m{}\x1b[39m \x1b[2m({})\x1b[22m", files.len(), Size(total)))?;
        }
      };

      Ok(())
    })
  }

  fn filter_and_count(&mut self, files: &mut Vec<File>) {
    let inputs_sig_count = files.iter().fold(SigCount::default(), |mut acc, &(_, sig)| {
      *acc.entry(sig).or_default() += 1;
      acc
    });

    let skip = files.extract_if(.., |&mut (_, sig @ [meta, _])| {
      let [hash, size] = [meta & HASH_MASK, meta & SIZE_MASK];
      size == 0 || // empty file
        size <= PARTIAL_HASH_SIZE && hash != 0 || // already fully hashed small file
        inputs_sig_count.get(&sig).is_some_and(|&n| n < 2) // unique file
    });

    self.count(skip);
  }

  fn count(&mut self, files: impl IntoIterator<Item = File>) {
    for (path, sig) in files {
      self.files.push((path, sig));
      *self.sig_count.entry(sig).or_default() += 1;
    }
  }

  fn compute_hashes(&mut self, files: &mut Vec<File>, hash: FileHash) -> io::Result<Stats> {
    let 1.. = files.len() else { return Ok(Default::default()) };

    let (name, marker_bit, max_bytes) = match hash {
      FileHash::Full => ("full", HASH_FULL, u64::MAX),
      FileHash::Prefix(_) => ("prefix", HASH_PREF, PARTIAL_HASH_SIZE),
      FileHash::Suffix(_) => ("suffix", HASH_SUFF, PARTIAL_HASH_SIZE),
    };

    let threads_n = thread::available_parallelism()?.get();
    let (inputs_tx, inputs_rx) = channel::bounded::<(Box<Path>, Sig)>(threads_n << 8);
    let (outputs_tx, outputs_rx) = channel::bounded::<(Box<Path>, io::Result<Sig>)>(threads_n << 8);

    // n workers
    for _ in 0..threads_n {
      let inputs_rx = inputs_rx.clone();
      let outputs_tx = outputs_tx.clone();
      thread::spawn(move || {
        for (path, [meta, _]) in inputs_rx {
          let sig = hash.compute(&path).map(|hash| [meta | marker_bit, hash]);
          let _ = outputs_tx.send((path, sig));
        }
      });
    }

    drop(inputs_rx);
    drop(outputs_tx);

    // producer
    let inputs = mem::take(files);
    let inputs_n = inputs.len();
    thread::spawn(move || {
      for input in inputs {
        let _ = inputs_tx.send(input);
      }
    });

    // consumer
    writeln!(self.w)?;
    writeln!(self.w, "computing \x1b[93m{inputs_n}\x1b[39m {name} hashes… \x1b[2m({threads_n} threads)\x1b[22m")?;
    let t0 = Instant::now();
    let mut progress = Progress::new(&mut self.w, format_args!("computed"))?;
    let mut bytes = 0;
    for (path, sig) in outputs_rx {
      match sig {
        Err(err) => self.errs.push((path, true, err)),
        Ok(sig @ [meta, _]) => {
          files.push((path, sig));
          bytes += max_bytes.min(meta & SIZE_MASK);
          progress.update(format_args!("\x1b[93m{}\x1b[39m", files.len()))?;
        }
      };
    }
    let t1 = Instant::now();

    Ok((inputs_n, bytes, t1.duration_since(t0)))
  }

  fn show_duplicates(&mut self) -> io::Result<()> {
    let mut path_groups = HashMap::<_, Vec<_>>::default();
    for &(ref path, sig @ [_, hash]) in &self.files {
      if hash != 0 && self.sig_count.get(&sig).is_some_and(|&n| n > 1) {
        path_groups.entry(sig).or_default().push(&**path);
      }
    }

    let mut path_groups = Vec::from_iter(path_groups);
    path_groups.sort_unstable_by(|([meta0, _], group0), ([meta1, _], group1)| {
      let a = (group0.len() as u64 - 1) * (meta0 & SIZE_MASK);
      let b = (group1.len() as u64 - 1) * (meta1 & SIZE_MASK);
      a.cmp(&b) // by total duplicated bytes
    });

    for ([meta, hash], mut group) in path_groups {
      let len = group.len();
      let mid = len.min(3);

      let size = meta & SIZE_MASK;
      let each = Size(size);
      let dup = Size(size * (len as u64 - 1));

      let hash = format_args!("\x1b[96m{hash:016x}\x1b[39m");
      let count = format_args!("\x1b[93m{len}\x1b[39m files");
      let each = format_args!("\x1b[93m{each}\x1b[39m each");
      let dup = format_args!("\x1b[93m{dup}\x1b[39m duplicated");

      writeln!(self.w)?;
      writeln!(self.w, "{hash}{0}{count}{0}{each}{0}{dup}", " \x1b[2m·\x1b[22m ")?;

      let cmp = |p0: &&Path, p1: &&Path| {
        let [c0, c1] = [p0, p1].map(|p| p.components().count());
        let [n0, n1] = [p0, p1].map(|p| p.as_os_str().len());
        (c0, n0, p0).cmp(&(c1, n1, p1)) // by depth, by length, lexicographically
      };

      let (show, hide) = {
        let (slice, _, _) = group.select_nth_unstable_by(mid - 1, cmp);
        slice.sort_unstable_by(cmp);
        group.split_at(mid)
      };

      for (i, path) in show.iter().enumerate() {
        let [ansi0, ansi1] = if i == 0 { ["", ""] } else { ["\x1b[2m", "\x1b[22m"] };
        let path = path.strip_prefix(&self.root).unwrap_or(path).display();
        writeln!(self.w, "{ansi0}{path}{ansi1}")?
      }

      if let n @ 1.. = hide.len() {
        writeln!(self.w, "\x1b[2mand {n} more…\x1b[22m")?;
      }
    }

    Ok(())
  }

  fn show_errors(&mut self) -> io::Result<()> {
    let 1.. = self.errs.len() else { return Ok(()) };

    self.errs.sort_unstable_by(|(p0, _, e0), (p1, _, e1)| {
      let a = (e0.kind(), e0.raw_os_error(), p0);
      let b = (e1.kind(), e1.raw_os_error(), p1);
      a.cmp(&b) // by error kind, by OS error code, lexicographically by path
    });

    let groups = self.errs.chunk_by(|(_, _, e0), (_, _, e1)| {
      let a = (e0.kind(), e0.raw_os_error());
      let b = (e1.kind(), e1.raw_os_error());
      a == b // by error kind and OS error code
    });

    for group in groups {
      let (_, _, err) = &group[0];
      writeln!(self.w)?;
      writeln!(self.w, "\x1b[91m{err}:\x1b[39m")?;

      let (show, hide) = group.split_at(group.len().min(3));

      for &(ref path, full_path, _) in show {
        let [ansi0, ansi1] = if full_path { ["", ""] } else { ["\x1b[2m", "\x1b[22m"] };
        let path = path.strip_prefix(&self.root).unwrap_or(path);
        let path = if path.as_os_str().len() == 0 { &self.root } else { path };
        writeln!(self.w, "{ansi0}{}{ansi1}", path.display())?
      }

      if let n @ 1.. = hide.len() {
        writeln!(self.w, "\x1b[2mand {n} more…\x1b[22m")?;
      }
    }

    Ok(())
  }

  fn show_summary(&mut self, [prefix, suffix, full]: [Stats; 3]) -> io::Result<()> {
    let (mut total_n, mut skipped_n, mut dup_n) = (0, 0, 0);
    let (mut total, mut skipped, mut dup) = (0, 0, 0);

    for (&[meta, _], &count) in &self.sig_count {
      let size = meta & SIZE_MASK;
      total_n += count;
      total += count * size;

      if count > 1 {
        dup_n += count - 1;
        dup += (count - 1) * size;
      }

      if meta & HASH_MASK == 0 {
        skipped_n += count;
        skipped += count * size;
      }
    }

    let percentage = |n, total| if total == 0 { 0. } else { 1e2 * n as f64 / total as f64 };

    let (uniq, uniq_n) = (total - dup, total_n - dup_n);
    let (uniq_pct, uniq_n_pct) = (percentage(uniq, total), percentage(uniq_n, total_n));
    let (dup_pct, dup_n_pct) = (percentage(dup, total), percentage(dup_n, total_n));
    let (total, uniq, dup, skipped) = (Size(total), Size(uniq), Size(dup), Size(skipped));

    {
      let total_n = format_args!("\x1b[96m{total_n}\x1b[39m");
      let uniq_n = format_args!("\x1b[92m{uniq_n}\x1b[39m \x1b[2m({uniq_n_pct:.0}%)\x1b[22m");
      let dup_n = format_args!("\x1b[93m{dup_n}\x1b[39m \x1b[2m({dup_n_pct:.0}%)\x1b[22m");
      writeln!(self.w)?;
      writeln!(self.w, "{total_n} files: {uniq_n} unique and {dup_n} duplicates")?;

      let total = format_args!("\x1b[96m{total}\x1b[39m");
      let uniq = format_args!("\x1b[92m{uniq}\x1b[39m \x1b[2m({uniq_pct:.0}%)\x1b[22m");
      let dup = format_args!("\x1b[93m{dup}\x1b[39m \x1b[2m({dup_pct:.0}%)\x1b[22m");
      writeln!(self.w)?;
      writeln!(self.w, "{total} of data: {uniq} unique and {dup} duplicated")?;
    }

    if total_n > skipped_n {
      writeln!(self.w)?;
      writeln!(self.w, "skipped \x1b[93m{skipped_n}\x1b[39m files \x1b[2m({skipped})\x1b[22m")?;

      for (name, (count, bytes, t)) in [("prefix", prefix), ("suffix", suffix), ("full", full)] {
        let s = t.as_secs_f64().max(f64::MIN_POSITIVE);
        let t = Time(t.as_nanos() as u64);

        let size = Size(bytes);
        let rate = Size((bytes as f64 / s) as u64);
        let rate_n = count as f64 / s;

        let main = format_args!("computed \x1b[93m{count}\x1b[39m {name} hashes in \x1b[93m{t}\x1b[39m");
        let extra = format_args!("\x1b[2m({rate_n:.0} files/s · {rate}/s · {size})\x1b[22m");
        writeln!(self.w, "{main} {extra}")?;
      }
    }

    Ok(())
  }
}
