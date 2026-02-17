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
    let ptr = self.as_mut_ptr();
    let len = self.len();
    let (mut r, mut w) = (0, 0);

    while r < len {
      let end = (r + 1..len) //.
        .find(|&i| unsafe { !same_bucket(&mut *ptr.add(r), &mut *ptr.add(i)) })
        .unwrap_or(len);

      if let 1 = end - r {
        unsafe { ptr.add(r).swap(ptr.add(w)) };
        w += 1;
      }

      r = end;
    }

    unsafe { self.split_at_mut_unchecked(w) }
  }
}
