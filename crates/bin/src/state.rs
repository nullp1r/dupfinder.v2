use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{io, io::prelude::*, mem, thread};

use stdx::fmt::{Size, Time};
use stdx::fs::{self, FileHash};
use stdx::slice::SliceExt;
use stdx::term::Progress;

use crossbeam_channel as channel;

type Sig = [u64; 2]; // [file size, file hash]
type File = (Box<Path>, Sig);
type Error = (Box<Path>, bool, io::Error); // `true` if full path
type Stats = (usize, u64, Duration); // (hashes, bytes, time taken)

const PATHS_PER_ENTRY: usize = 3;
const PARTIAL_HASH_SIZE: u64 = 1 << 12; // 4 KiB

pub struct State<W> {
  w: W,
  root: Box<Path>,
  errs: Vec<Error>,
  files: Vec<File>,
}

impl<W: Write> State<W> {
  pub fn new(w: W, root: impl Into<PathBuf>) -> Self {
    let root = root.into().into_boxed_path();
    let (errs, files) = Default::default();
    Self { w, root, errs, files }
  }

  pub fn run(mut self) -> io::Result<()> {
    let mut files = Default::default();

    self.scan(&mut files)?;
    self.filter_and_collect(&mut files);

    let prefix = self.compute_hashes(&mut files, FileHash::Prefix(PARTIAL_HASH_SIZE))?;
    self.filter_and_collect(&mut files);

    let suffix = self.compute_hashes(&mut files, FileHash::Suffix(PARTIAL_HASH_SIZE))?;
    self.filter_and_collect(&mut files);

    let full = self.compute_hashes(&mut files, FileHash::Full)?;
    self.collect(files);

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

  fn filter_and_collect(&mut self, files: &mut Vec<File>) {
    let filter = |&mut (_, [size, hash]): &mut File| {
      size == 0 || size <= PARTIAL_HASH_SIZE && hash != 0 // empty or fully hashed small files
    };

    files.sort_unstable_by_key(|&(_, sig)| sig);
    let (unique, _) = files.partition_unique_by_key(|&mut (_, sig)| sig);
    let (unique, duplicates) = (..unique.len(), unique.len()..);
    self.collect(files.extract_if(duplicates, filter));
    self.collect(files.drain(unique));
  }

  fn collect(&mut self, files: impl IntoIterator<Item = File>) {
    self.files.extend(files);
  }

  fn compute_hashes(&mut self, files: &mut Vec<File>, hash: FileHash) -> io::Result<Stats> {
    let 1.. = files.len() else { return Ok(Default::default()) };

    files.sort_unstable_by(|(path0, _), (path1, _)| path0.cmp(path1));

    let (hash_type, max_bytes) = match hash {
      FileHash::Full => ("full", u64::MAX),
      FileHash::Prefix(_) => ("prefix", PARTIAL_HASH_SIZE),
      FileHash::Suffix(_) => ("suffix", PARTIAL_HASH_SIZE),
    };

    let inputs = mem::take(files);
    let inputs_n = inputs.len();
    let threads_n = thread::available_parallelism()?.get();
    let (inputs_tx, inputs_rx) = channel::bounded::<(Box<Path>, Sig)>(threads_n << 8);
    let (outputs_tx, outputs_rx) = channel::bounded::<(Box<Path>, io::Result<Sig>)>(threads_n << 8);

    thread::scope(|scope| {
      // n workers
      for _ in 0..threads_n {
        let inputs_rx = inputs_rx.clone();
        let outputs_tx = outputs_tx.clone();
        scope.spawn(move || {
          for (path, [meta, _]) in inputs_rx {
            let sig = hash.compute(&path).map(|hash| [meta, hash]);
            let Ok(_) = outputs_tx.send((path, sig)) else { break };
          }
        });
      }
      drop(inputs_rx);
      drop(outputs_tx);

      // producer
      scope.spawn(move || {
        for input in inputs {
          let Ok(_) = inputs_tx.send(input) else { break };
        }
      });

      // consumer
      writeln!(self.w)?;
      writeln!(self.w, "computing \x1b[93m{inputs_n}\x1b[39m {hash_type} hashes… \x1b[2m({threads_n} threads)\x1b[22m")?;
      let mut progress = Progress::new(&mut self.w, format_args!("computed"))?;
      let mut bytes = 0;
      let t0 = Instant::now();
      for (path, sig) in outputs_rx {
        match sig {
          Err(err) => self.errs.push((path, true, err)),
          Ok(sig @ [size, _]) => {
            files.push((path, sig));
            bytes += max_bytes.min(size);
            progress.update(format_args!("\x1b[93m{}\x1b[39m", files.len()))?;
          }
        };
      }
      let t1 = Instant::now();

      Ok((inputs_n, bytes, t1.duration_since(t0)))
    })
  }

  fn show_duplicates(&mut self) -> io::Result<()> {
    let mut groups = Vec::from_iter({
      self.files.sort_unstable_by_key(|&(_, sig)| sig);
      self.files.chunk_by_mut(|&(_, sig0), &(_, sig1)| sig0 == sig1)
    });

    groups.sort_unstable_by_key(|paths| {
      let size = if let &&mut [(_, [size, _]), ..] = paths { size } else { 0 };
      (paths.len() as u64 - 1) * size // by total duplicated bytes
    });

    for paths in groups {
      let &mut [(_, [size, hash]), _, ..] = paths else { continue };
      let count = paths.len();
      let show = count.min(PATHS_PER_ENTRY);

      let (show, hide) = {
        let cmp = |(p0, _): &File, (p1, _): &File| {
          let [c0, c1] = [p0, p1].map(|p| p.components().count());
          let [n0, n1] = [p0, p1].map(|p| p.as_os_str().len());
          (c0, n0, p0).cmp(&(c1, n1, p1)) // by depth, by length, lexicographically
        };

        let (slice, _, _) = paths.select_nth_unstable_by(show - 1, cmp);
        slice.sort_unstable_by(cmp);
        paths.split_at(show)
      };

      {
        let each = Size(size);
        let dup = Size(size * (count as u64 - 1));

        let hash = format_args!("\x1b[96m{hash:016x}\x1b[39m");
        let count = format_args!("\x1b[93m{count}\x1b[39m files");
        let each = format_args!("\x1b[93m{each}\x1b[39m each");
        let dup = format_args!("\x1b[93m{dup}\x1b[39m duplicated");
        let sep = "\x1b[2m·\x1b[22m";

        writeln!(self.w)?;
        writeln!(self.w, "{hash} {sep} {count} {sep} {each} {sep} {dup}")?;
      }

      for (i, (path, _)) in show.iter().enumerate() {
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

    for paths in groups {
      let [(_, _, err), ..] = paths else { continue };
      let show = paths.len().min(PATHS_PER_ENTRY);
      let (show, hide) = paths.split_at(show);

      writeln!(self.w)?;
      writeln!(self.w, "\x1b[91m{err}:\x1b[39m")?;

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
    let (mut total_n, mut skipped_n, mut unique_n, mut delete_n) = (0, 0, 0, 0);
    let (mut total, mut skipped, mut unique, mut delete) = (0, 0, 0, 0);

    self.files.sort_unstable_by_key(|&(_, sig)| sig);
    let groups = self.files.chunk_by(|&(_, sig0), &(_, sig1)| sig0 == sig1);

    for group in groups {
      let &[(_, [size, hash]), ..] = group else { continue };
      let count = group.len() as u64;

      total_n += count;
      total += count * size;

      if let 0 = hash {
        skipped_n += count;
        skipped += count * size;
      }

      if let 1 = count {
        unique_n += count;
        unique += count * size;
      }

      if let 2.. = count {
        delete_n += count - 1;
        delete += (count - 1) * size;
      }
    }

    let percentage = |n, total| if total == 0 { 0. } else { 1e2 * n as f64 / total as f64 };

    let (keep, keep_n) = (total - unique - delete, total_n - unique_n - delete_n);
    let (unique_pct, unique_n_pct) = (percentage(unique, total), percentage(unique_n, total_n));
    let (keep_pct, keep_n_pct) = (percentage(keep, total), percentage(keep_n, total_n));
    let (delete_pct, delete_n_pct) = (percentage(delete, total), percentage(delete_n, total_n));
    let (total, unique, keep, delete, skipped) = (Size(total), Size(unique), Size(keep), Size(delete), Size(skipped));

    {
      let [wt, wu, wk, wd] = [total_n, unique_n, keep_n, delete_n].map(|n| 1 + n.max(1000).ilog10() as usize);
      let [w0, w1, w2, w3, w12] = [wt + 8, wu + 12, wk + 12, wd + 12, wu + wk + 25];

      let sep = "\x1b[2m│\x1b[22m";
      let top = format_args!("\x1b[2m┌{:─>w0$}┬{:─>w1$}─{:─>w2$}┬{:─>w3$}┐\x1b[22m", "", "", "", "");
      let mid = format_args!("\x1b[2m├{:─>w0$}┼{:─>w1$}┬{:─>w2$}┼{:─>w3$}┤\x1b[22m", "", "", "", "");
      let bot = format_args!("\x1b[2m└{:─>w0$}┴{:─>w1$}┴{:─>w2$}┴{:─>w3$}┘\x1b[22m", "", "", "", "");

      let total_n = format_args!(" \x1b[96m{total_n:wt$} files\x1b[39m ");
      let unique_n = format_args!(" \x1b[92m{unique_n:wu$} files\x1b[39;2m{unique_n_pct:3.0}%\x1b[22m ");
      let keep_n = format_args!(" \x1b[93m{keep_n:wk$} files\x1b[39;2m{keep_n_pct:3.0}%\x1b[22m ");
      let delete_n = format_args!(" \x1b[91m{delete_n:wd$} files\x1b[39;2m{delete_n_pct:3.0}%\x1b[22m ");

      let total = format_args!(" \x1b[96m{total:wt$}  \x1b[39m ");
      let unique = format_args!(" \x1b[92m{unique:wu$}  \x1b[39;2m{unique_pct:3.0}%\x1b[22m ");
      let keep = format_args!(" \x1b[93m{keep:wk$}  \x1b[39;2m{keep_pct:3.0}%\x1b[22m ");
      let delete = format_args!(" \x1b[91m{delete:wd$}  \x1b[39;2m{delete_pct:3.0}%\x1b[22m ");

      writeln!(self.w)?;
      writeln!(self.w, "{top}")?;
      writeln!(self.w, "{sep}{:^w0$}{sep}{:^w12$}{sep}{:^w3$}{sep}", "total", "unique and potentially unique", "duplicates")?;
      writeln!(self.w, "{mid}")?;
      writeln!(self.w, "{sep}{total_n}{sep}{unique_n}{sep}{keep_n}{sep}{delete_n}{sep}")?;
      writeln!(self.w, "{sep}{total}{sep}{unique}{sep}{keep}{sep}{delete}{sep}")?;
      writeln!(self.w, "{bot}")?;
    }

    if total_n > skipped_n {
      writeln!(self.w)?;
      writeln!(self.w, "\x1b[2mskipped {skipped_n} files ({skipped})\x1b[22m")?;

      for (name, (count, bytes, t)) in [("prefix", prefix), ("suffix", suffix), ("full", full)] {
        let s = t.as_secs_f64().max(f64::MIN_POSITIVE);
        let t = Time(t.as_nanos() as u64);

        let size = Size(bytes);
        let rate = Size((bytes as f64 / s) as u64);
        let rate_n = count as f64 / s;

        let count = format_args!("computed {count} {name} hashes in {t}");
        let stats = format_args!("({rate_n:.0} h/s · {rate}/s · {size})");
        writeln!(self.w, "\x1b[2m{count} {stats}\x1b[22m")?;
      }
    }

    Ok(())
  }
}
