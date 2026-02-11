use std::hash::BuildHasherDefault;
use std::{collections, hash};

pub type BuildHasher = BuildHasherDefault<Hasher>;
pub type HashMap<K, V> = collections::HashMap<K, V, BuildHasher>;
pub type HashSet<V> = collections::HashSet<V, BuildHasher>;

const PHI: u64 = 0x9e3779b97f4a7c15;

pub struct Hasher {
  state: u64,
}

impl Default for Hasher {
  fn default() -> Self {
    Self { state: PHI }
  }
}

impl Hasher {
  fn bytes(&mut self, input: &[u8]) {
    let (chunks, tail) = input.as_chunks();
    for &chunk in chunks {
      self.bits(u64::from_ne_bytes(chunk), 0);
    }
    if let n @ 1.. = tail.len() {
      self.bits(tail.iter().rev().fold(0, |a, &b| a << 8 | b as u64), n as u32);
    }
  }

  fn bits(&mut self, input: u64, rotate: u32) {
    self.state = (self.state ^ input).rotate_left(rotate).wrapping_mul(PHI);
  }

  fn mix(&self) -> u64 {
    let mut x = self.state;
    x = (x ^ x >> 27).wrapping_mul(0x3c79ac492ba7b653);
    x = (x ^ x >> 33).wrapping_mul(0x1c69b3f74ac4ae35);
    x ^ x >> 27
  }
}

impl hash::Hasher for Hasher {
  fn finish(&self) -> u64 {
    self.mix()
  }

  fn write(&mut self, input: &[u8]) {
    self.bytes(input);
  }

  fn write_u8(&mut self, input: u8) {
    self.bits(input as u64, 0);
  }

  fn write_u16(&mut self, input: u16) {
    self.bits(input as u64, 0);
  }

  fn write_u32(&mut self, input: u32) {
    self.bits(input as u64, 0);
  }

  fn write_u64(&mut self, input: u64) {
    self.bits(input, 0);
  }

  fn write_u128(&mut self, input: u128) {
    self.bits(input as u64, 0);
    self.bits((input >> 64) as u64, 0);
  }

  fn write_usize(&mut self, input: usize) {
    self.bits(input as u64, 0);
  }
}
