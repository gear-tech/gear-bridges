// This file is part of Gear.

// Copyright (C) 2024 Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! The module contains implementations of basic types used by higher level
//! types. Inspired by https://github.com/sigp/ssz_types/ and https://github.com/ralexstokes/ssz-rs.

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
