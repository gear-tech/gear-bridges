use super::*;

/// A homogenous collection of a variable number of values.
#[derive(Clone, TypeInfo)]
#[scale_info(bounds(T: TypeInfo))]
pub struct List<T, const N: usize> {
    data: Vec<T>,
}

struct ListVisitor<T>(PhantomData<Vec<T>>);

impl<'de, T: Deserialize<'de>> de::Visitor<'de> for ListVisitor<T> {
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("array of objects")
    }

    fn visit_seq<S>(self, visitor: S) -> Result<Self::Value, S::Error>
    where
        S: de::SeqAccess<'de>,
    {
        Deserialize::deserialize(de::value::SeqAccessDeserializer::new(visitor))
    }
}

impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for List<T, N> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let data = deserializer.deserialize_seq(ListVisitor(PhantomData))?;
        List::<T, N>::try_from(data).map_err(de::Error::custom)
    }
}

impl<T, const N: usize> AsRef<[T]> for List<T, N> {
    fn as_ref(&self) -> &[T] {
        &self.data
    }
}

impl<T: Debug, const N: usize> fmt::Debug for List<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        if f.alternate() {
            write!(
                f,
                "List<{}, {}>(len={}){:#?}",
                any::type_name::<T>(),
                N,
                self.len(),
                self.data
            )
        } else {
            write!(
                f,
                "List<{}, {}>(len={}){:?}",
                any::type_name::<T>(),
                N,
                self.len(),
                self.data
            )
        }
    }
}

impl<T, const N: usize> Default for List<T, N> {
    fn default() -> Self {
        let data = vec![];
        data.try_into()
            .expect("any List can be constructed from an empty Vec")
    }
}

impl<T: PartialEq, const N: usize> PartialEq for List<T, N> {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl<T, const N: usize> Eq for List<T, N> where T: Eq {}

impl<T, const N: usize> TryFrom<Vec<T>> for List<T, N> {
    type Error = String;

    fn try_from(data: Vec<T>) -> Result<Self, Self::Error> {
        if data.len() > N {
            let len = data.len();
            Err(format!(
                "Unable to construct List<T, {N}> from vec![T; {len}]"
            ))
        } else {
            Ok(Self { data })
        }
    }
}

impl<T: Clone, const N: usize> TryFrom<&[T]> for List<T, N> {
    type Error = String;

    fn try_from(data: &[T]) -> Result<Self, Self::Error> {
        if data.len() > N {
            let len = data.len();
            Err(format!(
                "Unable to construct List<T, {N}> from &[T] (length = {len})"
            ))
        } else {
            Ok(Self {
                data: data.to_vec(),
            })
        }
    }
}

impl<T: Clone, const N: usize> From<&[T; N]> for List<T, N> {
    fn from(value: &[T; N]) -> Self {
        Self {
            data: value.to_vec(),
        }
    }
}

impl<T, const N: usize> Deref for List<T, N> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

// NOTE: implement `IndexMut` rather than `DerefMut` to ensure
// the inner data is not mutated without being able to
// track which elements changed
impl<T, Idx: SliceIndex<[T]>, const N: usize> Index<Idx> for List<T, N> {
    type Output = <Idx as SliceIndex<[T]>>::Output;

    fn index(&self, index: Idx) -> &Self::Output {
        &self.data[index]
    }
}

impl<T, const N: usize> IndexMut<usize> for List<T, N> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.data[index]
    }
}

impl<T, const N: usize> List<T, N> {
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn push(&mut self, element: T) -> Option<()> {
        if self.data.len() < N {
            self.data.push(element);

            return Some(());
        }

        None
    }

    pub fn pop(&mut self) -> Option<T> {
        self.data.pop()
    }

    pub fn clear(&mut self) {
        self.data.clear();
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, T, N> {
        IterMut {
            inner: self.data.iter_mut(),
        }
    }
}

pub struct IterMut<'a, T, const N: usize> {
    inner: slice::IterMut<'a, T>,
}

impl<'a, T, const N: usize> Iterator for IterMut<'a, T, N> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<T: Decode, const N: usize> Decode for List<T, N> {
    fn decode<I: parity_scale_codec::Input>(
        input: &mut I,
    ) -> Result<Self, parity_scale_codec::Error> {
        let data = <Vec<T> as Decode>::decode(input)?;
        if data.len() >= N {
            return Err("Decoded Vec length is greater than N".into());
        }

        Ok(Self { data })
    }
}

impl<T: Encode, const N: usize> Encode for List<T, N> {
    fn encode(&self) -> Vec<u8> {
        self.data.encode()
    }

    fn encode_to<W: parity_scale_codec::Output + ?Sized>(&self, dest: &mut W) {
        self.data.encode_to(dest)
    }

    fn encoded_size(&self) -> usize {
        self.data.encoded_size()
    }

    fn size_hint(&self) -> usize {
        self.data.size_hint()
    }

    fn using_encoded<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
        self.data.using_encoded(f)
    }
}

impl<T: TreeHash, const N: usize> TreeHash for List<T, N> {
    fn tree_hash_type() -> TreeHashType {
        TreeHashType::List
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        unreachable!("List should never be packed.")
    }

    fn tree_hash_packing_factor() -> usize {
        unreachable!("List should never be packed.")
    }

    fn tree_hash_root(&self) -> Hash256 {
        let root = utils::vec_tree_hash_root::<T, N>(&self.data);

        tree_hash::mix_in_length(&root, self.len())
    }
}

#[test]
fn scale_codec_list() {
    const N: usize = 100;

    let mut list = List::<_, N>::default();

    for i in 0..N {
        list.push(i as u32);
    }

    let encoded = Encode::encode(&list);
    let decoded = List::decode(&mut &encoded[..]).unwrap();

    assert_eq!(list, decoded);
}
