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

use crate::units::UtcTime;
use std::collections::{btree_map, BTreeMap};
use std::iter;

/// A time-indexed map
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct TimeMap<V> {
    map: BTreeMap<(UtcTime, usize), V>,
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
    pub fn pop_first(&mut self) -> Option<(UtcTime, V)> {
        let first_key = self.map.keys().next().copied();
        let value = first_key.and_then(|key| self.map.remove(&key));
        first_key.map(|key| (key.0, value.unwrap()))
    }

    /// Pops the maximal element from the stack, according to some maximization function
    ///
    /// Unlike `pop_first` this function is O(n), and if you are using it heavily,
    /// it may make sense to change data structures.
    pub fn pop_max<F, T>(&mut self, mut maxfn: F) -> Option<(UtcTime, V)>
    where
        F: FnMut(&V) -> T,
        T: Ord,
    {
        let mut max_key_val = None;
        for (k, v) in &self.map {
            let new_max = maxfn(v);
            if let Some((ref mut key, ref mut max)) = max_key_val {
                if new_max > *max {
                    *key = *k;
                    *max = new_max;
                }
            } else {
                max_key_val = Some((*k, new_max));
            }
        }
        max_key_val.and_then(|(key, _)| self.map.remove(&key).map(|v| (key.0, v)))
    }

    /// Inserts a new element. Allows duplicates.
    ///
    /// There is no way to replace or delete an element once it is added to the
    /// time map. If you insert an element twice, even with the same timestamp,
    /// it will just be in the map twice.
    pub fn insert(&mut self, time: UtcTime, item: V) {
        let idx = self.next_idx;
        // If this assertion fails it means we somehow used `idx` twice
        assert!(self.map.insert((time, idx), item).is_none());
        self.next_idx += 1;
    }

    /// Returns the most recent element whose timestamp is prior to the given timestamp
    pub fn most_recent(&self, as_of: UtcTime) -> Option<(UtcTime, &V)> {
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
    iter: btree_map::Values<'a, (UtcTime, usize), V>,
}
impl<'a, V> Iterator for Values<'a, V> {
    type Item = &'a V;
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

/// Borrowed iterator over (timestamp, entry) pairs
pub struct Iter<'a, V> {
    iter: btree_map::Iter<'a, (UtcTime, usize), V>,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (UtcTime, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|((time, _), v)| (*time, v))
    }
}

impl<'a, V> iter::IntoIterator for &'a TimeMap<V> {
    type Item = (UtcTime, &'a V);
    type IntoIter = Iter<'a, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Owned iterator over (timestamp, entry) pairs
pub struct IntoIter<V> {
    iter: btree_map::IntoIter<(UtcTime, usize), V>,
}

impl<V> Iterator for IntoIter<V> {
    type Item = (UtcTime, V);
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|((time, _), v)| (time, v))
    }
}

impl<V> iter::IntoIterator for TimeMap<V> {
    type Item = (UtcTime, V);
    type IntoIter = IntoIter<V>;
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            iter: self.map.into_iter(),
        }
    }
}
