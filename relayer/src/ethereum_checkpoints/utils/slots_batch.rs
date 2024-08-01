use checkpoint_light_client_io::Slot;

/// Iterator produces right open intervals of the specific size in backward direction.
pub struct Iter {
    slot_start: Slot,
    slot_end: Slot,
    batch_size: Slot,
}

impl Iter {
    pub fn new(slot_start: Slot, slot_end: Slot, batch_size: Slot) -> Option<Self> {
        if batch_size < 2 || slot_start >= slot_end {
            return None;
        }

        Some(Self {
            slot_start,
            slot_end,
            batch_size,
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

    // [9; 10), [8; 9), etc
    assert_eq!(iter.next(), Some((9, 10)));
    assert_eq!(iter.next(), Some((8, 9)));
    assert_eq!(iter.next(), Some((7, 8)));
    assert_eq!(iter.next(), Some((6, 7)));
    assert_eq!(iter.next(), Some((5, 6)));
    assert_eq!(iter.next(), Some((4, 5)));
    assert_eq!(iter.next(), Some((3, 4)));
    assert!(iter.next().is_none());

    let mut iter = Iter::new(3, 10, 3).unwrap();

    // [8; 10), [6; 8), [4; 6), [3; 4)
    assert_eq!(iter.next(), Some((8, 10)));
    assert_eq!(iter.next(), Some((6, 8)));
    assert_eq!(iter.next(), Some((4, 6)));
    assert_eq!(iter.next(), Some((3, 4)));
    assert!(iter.next().is_none());
}
