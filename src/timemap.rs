// Trade Tracker
// Written in 2021 by
//   Andrew Poelstra <tradetracker@wpsoftware.net>
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

//! Time Map
//!
//! An ordered set of elements, indexed by timestamp but where duplicate
//! timestamps are allowed (in which case the first-inserted ones will come
//! first).
//!
//! Supports iteration and popping from the front, but otherwise does not
//! support direct indexing or random access.
//!

use std::collections::{btree_map, BTreeMap};
use std::iter;
use time::OffsetDateTime;

/// A time-indexed map
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct TimeMap<V> {
    map: BTreeMap<(OffsetDateTime, usize), V>,
    next_idx: usize,
}

// Cannot be derived because the #derive logic is dumb and wants a
// Default bound on V even though we do not need one
impl<V> Default for TimeMap<V> {
    fn default() -> Self {
        TimeMap {
            map: Default::default(),
            next_idx: Default::default(),
        }
    }
}

impl<V> TimeMap<V> {
    /// Constructs a new empty time map
    pub fn new() -> Self {
        Default::default()
    }

    /// Computes the number of stored entries
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether or not the map is empty
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Pops the first element from the map, if one exists
    pub fn pop_first(&mut self) -> Option<(OffsetDateTime, V)> {
        let first_key = self.map.keys().next().map(|x| *x);
        let value = first_key.and_then(|key| self.map.remove(&key));
        first_key.map(|key| (key.0, value.unwrap()))
    }

    /// Inserts a new element. Allows duplicates.
    ///
    /// There is no way to replace or delete an element once it is added to the
    /// time map. If you insert an element twice, even with the same timestamp,
    /// it will just be in the map twice.
    pub fn insert(&mut self, time: OffsetDateTime, item: V) {
        let idx = self.next_idx;
        // If this assertion fails it means we somehow used `idx` twice
        assert!(self.map.insert((time, idx), item).is_none());
        self.next_idx += 1;
    }

    /// Returns the most recent element whose timestamp is prior to the given timestamp
    pub fn most_recent(&self, as_of: OffsetDateTime) -> Option<(OffsetDateTime, &V)> {
        self.map
            .range(..(as_of, 0))
            .rev()
            .next()
            .map(|((k, _), v)| (*k, v))
    }

    /// Constructs a borrowed iterator over the (time, value) pairs
    pub fn iter(&self) -> Iter<V> {
        Iter {
            iter: self.map.iter(),
        }
    }

    /// Constructs an owned iterator over the (time, value) pairs
    pub fn into_iter(self) -> IntoIter<V> {
        IntoIter {
            iter: self.map.into_iter(),
        }
    }

    /// Constructs a borrowed iterator over values in the map
    pub fn values(&self) -> Values<V> {
        Values {
            iter: self.map.values(),
        }
    }
}

// Iterators

/// Borrowed iterator overentries
pub struct Values<'a, V> {
    iter: btree_map::Values<'a, (OffsetDateTime, usize), V>,
}
impl<'a, V> Iterator for Values<'a, V> {
    type Item = &'a V;
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

/// Borrowed iterator over (timestamp, entry) pairs
pub struct Iter<'a, V> {
    iter: btree_map::Iter<'a, (OffsetDateTime, usize), V>,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (OffsetDateTime, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|((time, _), v)| (*time, v))
    }
}

impl<'a, V> iter::IntoIterator for &'a TimeMap<V> {
    type Item = (OffsetDateTime, &'a V);
    type IntoIter = Iter<'a, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Owned iterator over (timestamp, entry) pairs
pub struct IntoIter<V> {
    iter: btree_map::IntoIter<(OffsetDateTime, usize), V>,
}

impl<V> Iterator for IntoIter<V> {
    type Item = (OffsetDateTime, V);
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|((time, _), v)| (time, v))
    }
}

impl<V> iter::IntoIterator for TimeMap<V> {
    type Item = (OffsetDateTime, V);
    type IntoIter = IntoIter<V>;
    fn into_iter(self) -> Self::IntoIter {
        self.into_iter()
    }
}
