//! The module contains implementations of basic types used by higher level
//! types. Inspired by <https://github.com/sigp/ssz_types> and <https://github.com/ralexstokes/ssz-rs>.

use super::*;

mod bits;
mod byte_list;
mod bytes_fixed;
mod fixed_array;
mod list;

pub use bits::{List as Bitlist, Vector as Bitvector};
pub use byte_list::ByteList;
pub use bytes_fixed::BytesFixed;
pub use fixed_array::FixedArray;
pub use list::List;
