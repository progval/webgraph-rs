/*
 * SPDX-FileCopyrightText: 2023 Inria
 *
 * SPDX-License-Identifier: Apache-2.0 OR LGPL-2.1-or-later
 */

//! Collection of common functions we use throughout the codebase

use std::path::{Path, PathBuf};

mod dbg_codes;
pub use dbg_codes::*;

/// Bijective mapping from isize to u64 as defined in <https://github.com/vigna/dsiutils/blob/master/src/it/unimi/dsi/bits/Fast.java>
pub const fn int2nat(x: i64) -> u64 {
    (x << 1 ^ (x >> 63)) as u64
}

/// Bijective mapping from u64 to i64 as defined in <https://github.com/vigna/dsiutils/blob/master/src/it/unimi/dsi/bits/Fast.java>
///
/// ```
/// # use webgraph::utils::*;
///
/// assert_eq!(nat2int(0), 0);
/// assert_eq!(nat2int(1), -1);
/// assert_eq!(nat2int(2), 1);
/// assert_eq!(nat2int(3), -2);
/// assert_eq!(nat2int(4), 2);
/// ```
pub const fn nat2int(x: u64) -> i64 {
    ((x >> 1) ^ !((x & 1).wrapping_sub(1))) as i64
}

/// Appends a string to a path
///
/// ```
/// # use std::path::{Path, PathBuf};
/// # use webgraph::prelude::suffix_path;
///
/// assert_eq!(
///     suffix_path(Path::new("/tmp/graph"), "-transposed"),
///     Path::new("/tmp/graph-transposed").to_owned()
/// );
/// ```
#[inline(always)]
pub fn suffix_path<P: AsRef<Path>, S: AsRef<std::ffi::OsStr>>(path: P, suffix: S) -> PathBuf {
    let mut path = path.as_ref().as_os_str().to_owned();
    path.push(suffix);
    path.into()
}

mod circular_buffer;
pub(crate) use circular_buffer::*;

mod mmap_backend;
pub use mmap_backend::*;

mod perm;
pub use perm::*;

//mod sorted_graph;
//pub use sorted_graph::*;

mod kary_heap;
pub use kary_heap::*;

mod sort_pairs;
pub use sort_pairs::*;
