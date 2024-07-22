use checkpoint_light_client_io::Slot;

/// Iterator produces right open intervals of the specific size in backward direction.
pub struct Iter {
    slot_start: Slot, slot_end: Slot, batch_size: Slot,
}

impl Iter {
    pub fn new(slot_start: Slot, slot_end: Slot, batch_size: Slot) -> Option<Self> {
        if batch_size < 2 || slot_start >= slot_end {
            return None;
        }

        Some(Self {
            slot_start, slot_end, batch_size,
        })
    }
}

impl Iterator for Iter {
    // [slot_start; slot_end)
    type Item = (Slot, Slot);

    fn next(&mut self) -> Option<Self::Item> {
        if self.slot_start + self.batch_size <= self.slot_end {
            let slot_start = self.slot_end - self.batch_size + 1;
            let slot_end = self.slot_end;

            self.slot_end = slot_start;

            return Some((slot_start, slot_end));
        }

        if self.slot_start < self.slot_end {
            let slot_end = self.slot_end;

            self.slot_end = self.slot_start;

            return Some((self.slot_start, slot_end));
        }

        None
    }
}

#[test]
fn test_slots_batch_iterator() {
    assert!(Iter::new(3, 10, 0).is_none());
    assert!(Iter::new(3, 10, 1).is_none());
    assert!(Iter::new(3, 3, 2).is_none());
    assert!(Iter::new(10, 3, 2).is_none());
    assert!(Iter::new(10, 3, 0).is_none());

    let mut iter = Iter::new(3, 10, 2).unwrap();

    assert!(matches!(iter.next(), Some((start, end)) if start == 9 && end == 10));
    assert!(matches!(iter.next(), Some((start, end)) if start == 7 && end == 8));
    assert!(matches!(iter.next(), Some((start, end)) if start == 5 && end == 6));
    assert!(matches!(iter.next(), Some((start, end)) if start == 3 && end == 4));
    assert!(iter.next().is_none());

    let mut iter = Iter::new(3, 10, 3).unwrap();

    assert!(matches!(iter.next(), Some((start, end)) if start == 8 && end == 10));
    assert!(matches!(iter.next(), Some((start, end)) if start == 5 && end == 7));
    assert!(matches!(iter.next(), Some((start, end)) if start == 3 && end == 4));
    assert!(iter.next().is_none());
}
