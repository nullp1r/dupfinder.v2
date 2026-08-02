#![allow(uncommon_codepoints, mixed_script_confusables)]

use std::hash::{BuildHasher, Hasher as _};
use std::time::Instant;
use std::{array, fmt, hint, iter};

use stdx::fmt as fmtʹ;

use self::histogram::Histogram;
use self::stats::Welford;

mod color;
mod cubehelix;
mod histogram;
mod stats;
mod util;

fn main() {
  let hasher = stdx::hash::BuildHasher::default();

  util::header("COLORS SHOWCASE");
  colors();

  util::header("THROUGHPUT");
  throughput(&hasher);

  util::header("SEQUENCES: xx");
  sequences(&hasher, |xx, _| xx);

  util::header("SEQUENCES: xx + i");
  sequences(&hasher, |xx, i| xx.wrapping_add(i as u8));

  util::header("SEQUENCES: xx + i/8");
  sequences(&hasher, |xx, i| xx.wrapping_add((i / 8) as u8));

  util::header("SEQUENTIAL INPUTS: i++");
  sequential(&hasher);

  util::header("BUCKET DISTRIBUTION");
  distribution(&hasher);

  util::header("AVALANCHE MATRIX");
  avalanche(&hasher);
}

fn throughput(hasher: &impl BuildHasher) {
  const S: usize = 1000;
  const K: usize = 100;
  const N: usize = 0x180;

  let inputs: [_; N] = array::from_fn(|i| i as u8);
  let inputs: [_; N] = array::from_fn(|i| &inputs[..=i]);

  let throughputs = inputs.map(|input| {
    let samples: [_; S] = array::from_fn(|_| {
      let t = Instant::now();
      let mut h = hasher.build_hasher();
      for _ in 0..K {
        h.write(hint::black_box(input));
      }
      hint::black_box(h);
      let t = Instant::now().duration_since(t);
      let n = K * input.len(); // total bytes hashed
      1e-9 * n as f64 / t.as_secs_f64() // GiB/s
    });

    let finite = samples.into_iter().filter(|n| n.is_finite());
    Welford::from_iter(finite).stats(false)
  });

  let [[min, ..], [_median, ..], [max, ..]] = {
    let mut sorted = throughputs;
    sorted.sort_unstable_by(|[a, ..], [b, ..]| a.total_cmp(b));
    [sorted[0], sorted[sorted.len() / 2], sorted[sorted.len() - 1]]
  };

  print!("\x1b[2m");
  for _ in 0..8 {
    print!("    GiB/s     ");
  }
  println!("\x1b[22m");

  for (i, (input, [mean, _, std_dev])) in inputs.into_iter().zip(throughputs.into_iter()).enumerate() {
    print!("\x1b[2m{:3?}\x1b[22m ", input.len());
    let prec = fmtʹ::precision::<3>(mean);
    let [r, g, b] = cubehelix::standard(((mean - min) / (max - min)) as f32);
    print!("\x1b[38;2;{r};{g};{b}m{mean:.prec$}\x1b[39m ");
    let [r, g, b] = cubehelix::standard((std_dev / mean) as f32);
    print!("\x1b[38;2;{r};{g};{b}m±{std_dev:3.1}\x1b[39m");
    if let 7 = i % 8 { println!() } else { print!(" ") }
  }
}

fn sequences(hasher: &impl BuildHasher, seq: impl Fn(u8, usize) -> u8) {
  const N: usize = 0x100;

  let mut all = Vec::default();

  for byte in 0..=0xff {
    let inputs: [_; N] = array::from_fn(|i| seq(byte, i));
    let inputs: [_; N] = array::from_fn(|i| &inputs[..=i]);

    let mut outputs = inputs.map(|input| {
      let mut h = hasher.build_hasher();
      h.write(input);
      (input, h.finish())
    });

    all.extend(outputs.iter().map(|&(bytes, hash)| (hash, byte, bytes.len())));

    outputs.sort_unstable_by_key(|&(_, hash)| hash);
    let unique = outputs.chunk_by(|&(_, hash0), &(_, hash1)| hash0 == hash1).count();
    let color = if unique < N { 1 } else { 2 };
    print!("\x1b[2m{byte:02x?}\x1b[22m \x1b[9{color}m{unique:3}\x1b[39m");
    if let 15 = byte % 16 { println!() } else { print!(" ") }
  }
  println!();

  all.sort_unstable();
  let chunks = all.chunk_by(|&(hash0, ..), &(hash1, ..)| hash0 == hash1);
  let unique = chunks.clone().count();

  if unique < all.len() {
    let seq = &seq;
    let bytes = |byte, n| {
      fmt::from_fn(move |f| {
        let mut iter = 0..n;
        for i in (&mut iter).take(40) {
          write!(f, "{:02x}", seq(byte, i))?;
        }
        if let n @ 1.. = iter.count() {
          write!(f, "\x1b[2m… {n:3} more\x1b[22m")?;
        }
        Ok(())
      })
    };

    for chunk in chunks {
      if let &[(hash, ..), _, ..] = chunk {
        print!("\x1b[2m{hash:016x}\x1b[22m");
        for (i, &(_, byte, n)) in chunk.iter().enumerate() {
          let pad = if let 0 = i { 0 } else { 16 };
          let bytes = bytes(byte, n);
          println!("{:pad$} \x1b[93m{n:3}\x1b[39m {bytes}", "");
        }
      }
    }
    println!();
  }

  let color = if unique < all.len() { 1 } else { 2 };
  println!("total unique: \x1b[9{color}m{unique}\x1b[39m \x1b[2m/ {}\x1b[22m", all.len());
}

fn sequential(hasher: &impl BuildHasher) {
  const N: usize = 0x100000;

  let mut outputs = (0..N) //.
    .map(|i| (i, hasher.hash_one(i as u64)))
    .collect::<Vec<_>>();

  outputs.sort_unstable_by_key(|&(i, hash)| (hash, i));

  for window in util::windows::<3, _>(&outputs, 32) {
    for (j, (i, hash)) in window.into_iter().enumerate() {
      let [rʹ, gʹ, bʹ, .., b, g, r] = hash.to_ne_bytes();
      print!("\x1b[2m{i:05x}\x1b[22m {hash:016x} \x1b[38;2;{r};{g};{b}m██\x1b[38;2;{rʹ};{gʹ};{bʹ}m██\x1b[39m");
      if let 3 = j % 4 { println!() } else { print!(" ") }
    }
    println!();
  }

  let unique = outputs.chunk_by(|&(_, a), &(_, b)| a == b).count();
  let color = if unique < N { 1 } else { 2 };
  println!("unique: \x1b[9{color}m{unique}\x1b[39m \x1b[2m/ {N}\x1b[22m");
}

fn distribution(hasher: &impl BuildHasher) {
  const N: usize = 0x10000;
  const D: usize = 0x100;

  let meanʹ = (D * D * D) as f64 / N as f64;
  let varianceʹ = meanʹ * (1. - 1. / N as f64);
  let std_devʹ = varianceʹ.sqrt();

  let mut w = Welford::default();
  let mut h = Histogram::<21>::centered(255., 50.);

  let mut buckets = (0..N) //.
    .map(|i| (i, 0u32))
    .collect::<Vec<_>>();

  for z in 0..D {
    for y in 0..D {
      for x in 0..D {
        let hash = hasher.hash_one((x, y, z));
        let bucket = hash as usize % N;
        let (_, count) = &mut buckets[bucket];
        *count += 1;
      }
    }
  }

  buckets.sort_unstable_by_key(|&(i, count)| (count, i));

  h.extend(buckets.iter().map(|&(_, n)| n as f64));
  w.extend(buckets.iter().map(|&(_, n)| n as f64));
  let [mean, variance, std_dev] = w.stats(false);
  let std_dev_err = (std_dev - std_devʹ) / std_devʹ;

  for window in util::windows::<3, _>(&buckets, 64) {
    for (j, &(i, count)) in window.iter().enumerate() {
      let err = count as f64 - mean;
      let [r, g, b] = cubehelix::split((err / std_dev / util::p(0.999999)) as f32, 1.);
      print!("\x1b[2m{i:04x}\x1b[22m \x1b[38;2;{r};{g};{b}m{:+7.3}%\x1b[39m", 1e2 * err / mean);
      if let 7 = j % 8 { println!() } else { print!(" ") }
    }
    println!();
  }

  let used = buckets.iter().filter(|&&(_, count)| count != 0).count();

  // let empty_prob = (1. - 1. / N as f64).powi((D * D * D) as i32);
  // let emptyʹ = N as f64 * empty_prob;
  // let empty = (N - used) as f64;
  // let empty_err = (empty - emptyʹ) / emptyʹ;

  let chi_sq = (N as f64 * variance) / mean;
  let chi_sq_mean = (N - 1) as f64; // degrees of freedom
  let chi_sq_std_dev = (2. * chi_sq_mean).sqrt();
  let chi_sq_z = (chi_sq - chi_sq_mean) / chi_sq_std_dev;
  let chi_sq_color = if let -2.0..=2.0 = chi_sq_z { 2 } else { 1 };

  let fmt_used = format_args!("buckets used: \x1b[93m{used}\x1b[39m \x1b[2m/ {N}\x1b[22m");
  let fmt_mean = format_args!("μ: \x1b[93m{mean:.1}\x1b[39m");
  let fmt_std_dev = format_args!("σ: \x1b[93m{std_dev:.2}\x1b[39m \x1b[2m(err: {:+.2}%)\x1b[22m", 1e2 * std_dev_err);
  let fmt_chi = format_args!("χ²: \x1b[9{chi_sq_color}m{chi_sq:.1}\x1b[39m \x1b[2m(dof: {chi_sq_mean}, z: {chi_sq_z:+.2})\x1b[22m");
  println!("{fmt_used}{0}{fmt_mean}{0}{fmt_std_dev}{0}{fmt_chi}", " \x1b[2m·\x1b[22m ");
  println!();

  print!("{h}");
}

fn avalanche(hasher: &impl BuildHasher) {
  const N: usize = 0x10000;

  let mut matrix = [[0; 64]; 64];
  let mut rng = 1u64;
  let mut rng = || {
    rng = rng.wrapping_mul(0x5851f42d4c957f2d).wrapping_add(1);
    rng
  };

  for _ in 0..N {
    let input = rng();
    let base = hasher.hash_one(input);
    for in_bit in 0..64 {
      let diff = base ^ hasher.hash_one(input ^ 1 << in_bit);
      for out_bit in 0..64 {
        matrix[in_bit][out_bit] += (diff >> out_bit & 1) as u16;
      }
    }
  }

  let mean = N as f64 / 2.;
  let std_dev = (N as f64 / 4.).sqrt();

  print!("{:3}\x1b[2m", "");
  for i in 0..64 {
    match i % 4 {
      1 => {}
      0 => print!("{i:<2}"),
      _ => print!(" "),
    }
  }
  println!("\x1b[22m");

  let (chunks, _) = matrix.as_chunks();
  for (i, &[row0, row1]) in chunks.iter().enumerate() {
    print!("\x1b[2m{:2}\x1b[22m ", 2 * i);
    for (fg, bg) in iter::zip(row0, row1) {
      let [[r, g, b], [rʹ, gʹ, bʹ]] = [fg, bg].map(|count| {
        let err = count as f64 - mean;
        cubehelix::split((err / std_dev / util::p(0.999999)) as f32, -1.)
      });
      print!("\x1b[38;2;{r};{g};{b};48;2;{rʹ};{gʹ};{bʹ}m▀");
    }
    println!("\x1b[39;49m");
  }
}

fn colors() {
  for y in (0..5).map(|i| i as f32).map(|i| [(i + 0.25) / 5., (i + 0.75) / 5.]) {
    for x in (0..=110).map(|i| i as f32).map(|i| i / 110.) {
      let [[r, g, b], [rʹ, gʹ, bʹ]] = y.map(|y| cubehelix::hsl([x, 1., 1. - y]));
      print!("\x1b[38;2;{r};{g};{b};48;2;{rʹ};{gʹ};{bʹ}m▀");
    }
    println!("\x1b[39;49m");
  }
  println!();

  for sign in [-1., 1.] {
    for t in (0..=110).map(|i| i as f32 / 110.) {
      let [r, g, b] = cubehelix::split(2. * t - 1., sign);
      print!("\x1b[48;2;{r};{g};{b}m ");
    }
    println!("\x1b[49m");
  }

  for t in (0..=110).map(|i| i as f32 / 110.) {
    let [r, g, b] = cubehelix::standard(t);
    print!("\x1b[48;2;{r};{g};{b}m ");
  }
  println!("\x1b[49m");
}
