use std::hash::BuildHasherDefault;
use std::{array, collections, hash};

pub type BuildHasher = BuildHasherDefault<Hasher>;
pub type HashMap<K, V> = collections::HashMap<K, V, BuildHasher>;
pub type HashSet<V> = collections::HashSet<V, BuildHasher>;

pub struct Hasher {
  state: u64,
}

impl Default for Hasher {
  fn default() -> Self {
    Self { state: PHI }
  }
}

impl hash::Hasher for Hasher {
  fn finish(&self) -> u64 {
    mix(self.state)
  }

  fn write(&mut self, input: &[u8]) {
    self.state = hash_n(self.state, input);
  }

  fn write_u8(&mut self, input: u8) {
    self.state = hash_8(self.state, input as u64, 1);
  }

  fn write_u16(&mut self, input: u16) {
    self.state = hash_8(self.state, input as u64, 2);
  }

  fn write_u32(&mut self, input: u32) {
    self.state = hash_8(self.state, input as u64, 4);
  }

  fn write_u64(&mut self, input: u64) {
    self.state = hash_8(self.state, input, 8);
  }

  fn write_u128(&mut self, input: u128) {
    self.state = hash_8(self.state, input as u64, 0);
    self.state = hash_8(self.state, (input >> 64) as u64, 16);
  }

  fn write_usize(&mut self, input: usize) {
    self.state = hash_8(self.state, input as u64, size_of::<usize>() as u32);
  }
}

const PHI: u64 = 0x9e3779b97f4a7c15;

#[inline]
fn mix(mut acc: u64) -> u64 {
  acc = (acc ^ acc >> 27).wrapping_mul(0x3c79ac492ba7b653);
  acc = (acc ^ acc >> 33).wrapping_mul(0x1c69b3f74ac4ae35);
  acc ^ acc >> 27
}

#[inline]
fn hash_8(acc: u64, input: u64, id: u32) -> u64 {
  let rot = if let 0 = id { 0 } else { 1 + 2 * id };
  (acc ^ input).rotate_right(rot).wrapping_mul(PHI)
}

#[inline]
fn hash_n(acc: u64, input: &[u8]) -> u64 {
  let (blocks, tail) = input.as_chunks();
  let acc = hash_by::<0x10, 0x80>(acc, blocks);
  let (chunks, tail) = tail.as_chunks();
  let acc = hash_by8(acc, chunks);
  hash_tail(acc, tail)
}

#[inline]
fn hash_by<const L: usize, const B: usize>(acc: u64, blocks: &[[u8; B]]) -> u64 {
  assert_eq!(8 * L, B); // compile-time check
  let 1.. = blocks.len() else { return acc };
  let lanes = array::from_fn(|i| PHI.wrapping_mul((i + 2) as u64));
  let lanes = blocks.iter().fold(lanes, |lanes: [_; L], block: &[_; B]| {
    let (chunks, _) = block.as_chunks();
    array::from_fn(|i| hash_8(lanes[i], u64::from_ne_bytes(chunks[i]), 0))
  });
  lanes.into_iter().fold(acc, |a, b| a ^ b)
}

#[inline]
fn hash_by8(acc: u64, chunks: &[[u8; 8]]) -> u64 {
  chunks.iter().fold(acc, |acc, &bytes| hash_8(acc, u64::from_ne_bytes(bytes), 0))
}

#[inline]
fn hash_tail(acc: u64, bytes: &[u8]) -> u64 {
  let n @ 1.. = bytes.len() else { return acc };
  hash_8(acc, fold_tail(bytes), n as u32)
}

#[inline]
fn fold_tail(tail: &[u8]) -> u64 {
  let (u32, carry) = tail.as_chunks();
  let (u16, u8) = carry.as_chunks();
  let mut acc = 0;
  if let &[u32] = u32 {
    let u64 = u32::from_ne_bytes(u32) as u64;
    acc |= u64 << 8 * (0b000 & tail.len());
  }
  if let &[u16] = u16 {
    let u64 = u16::from_ne_bytes(u16) as u64;
    acc |= u64 << 8 * (0b100 & tail.len());
  }
  if let &[u8] = u8 {
    let u64 = u8 as u64;
    acc |= u64 << 8 * (0b110 & tail.len());
  }
  acc
}
