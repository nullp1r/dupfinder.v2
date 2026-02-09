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
