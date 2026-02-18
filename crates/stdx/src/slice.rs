pub trait SliceExt<T> {
  fn partition_unique(&mut self) -> (&mut [T], &mut [T])
  where
    T: PartialEq,
  {
    self.partition_unique_by(|a, b| a == b)
  }

  fn partition_unique_by_key<K, F>(&mut self, mut key: F) -> (&mut [T], &mut [T])
  where
    F: FnMut(&mut T) -> K,
    K: PartialEq,
  {
    self.partition_unique_by(|a, b| key(a) == key(b))
  }

  fn partition_unique_by<F>(&mut self, same_bucket: F) -> (&mut [T], &mut [T])
  where
    F: FnMut(&mut T, &mut T) -> bool;
}

impl<T> SliceExt<T> for [T] {
  fn partition_unique_by<F>(&mut self, mut same_bucket: F) -> (&mut [T], &mut [T])
  where
    F: FnMut(&mut T, &mut T) -> bool,
  {
    assert!(size_of::<T>() != 0, "ZSTs are not supported");

    unsafe {
      let start = self.as_mut_ptr();
      let end = start.add(self.len());
      let (mut read, mut write) = (start, start);

      while read < end {
        let mut cursor = read.add(1);
        while cursor < end && same_bucket(&mut *read, &mut *cursor) {
          cursor = cursor.add(1);
        }
        if let 1 = cursor.offset_from(read) {
          write.swap(read);
          write = write.add(1);
        }
        read = cursor;
      }

      let mid = write.offset_from(start) as usize;
      self.split_at_mut_unchecked(mid)
    }
  }
}
