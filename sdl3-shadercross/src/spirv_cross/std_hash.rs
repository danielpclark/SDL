// Part of the Rust translation of SPIRV-Cross; see LICENSE.txt.

//! `std::unordered_map<uint32_t, T>` and `std::unordered_set<uint32_t>`
//! with the iteration order of libstdc++ (GCC's standard library, which
//! the upstream build this translation is checked against uses).
//!
//! SPIRV-Cross iterates some of its unordered containers while emitting
//! code (for instance the variables of a function, to pick the block that
//! declares each), so their order shows in the output. libstdc++'s
//! `_Hashtable` is a singly linked list of nodes threaded through the
//! buckets: a node goes in front of the first node of its bucket, or at
//! the front of the whole list when its bucket is empty, and a rehash
//! relinks the nodes in list order the same way. The hash of an integer is
//! the integer itself, and the prime rehash policy (max load factor 1)
//! picks the bucket counts. This is that structure, with a `HashMap` index
//! for the lookups.

use std::collections::HashMap;

const NIL: usize = usize::MAX;
const BEFORE_BEGIN: usize = usize::MAX - 1;

#[derive(Clone, Debug)]
struct Node<T> {
    key: u32,
    value: T,
    next: usize,
}

/// `std::unordered_map<uint32_t, T>` in libstdc++'s iteration order.
#[derive(Clone, Debug)]
pub struct StdHashMap<T> {
    nodes: Vec<Node<T>>,
    // The free slots of `nodes` (erased nodes).
    free: Vec<usize>,
    head: usize,
    // For each bucket, the node before its first node (BEFORE_BEGIN when
    // that is the list head), or NIL when empty.
    buckets: Vec<usize>,
    next_resize: usize,
    index: HashMap<u32, usize>,
}

impl<T> Default for StdHashMap<T> {
    fn default() -> Self {
        StdHashMap {
            nodes: Vec::new(),
            free: Vec::new(),
            head: NIL,
            buckets: vec![NIL],
            next_resize: 0,
            index: HashMap::new(),
        }
    }
}

// The bucket counts libstdc++'s __prime_list (hashtable-aux.cc) gives a
// growing table: each rehash asks for at least twice the buckets, and these
// are the primes of the list that answers (from 13 on; the fast table
// covers the smaller counts).
const STD_PRIMES: [usize; 19] = [
    13, 29, 59, 127, 257, 541, 1109, 2357, 5087, 10273, 20753, 42043, 85229, 172933, 351061,
    712697, 1447153, 2938679, 5967347,
];

fn is_prime(p: usize) -> bool {
    p >= 2 && (2..).take_while(|d| d * d <= p).all(|d| p % d != 0)
}

impl<T> StdHashMap<T> {
    pub fn new() -> Self {
        Self::default()
    }

    // _Prime_rehash_policy::_M_next_bkt().
    fn next_bkt(&mut self, n: usize) -> usize {
        const FAST_BKT: [usize; 14] = [2, 2, 2, 3, 5, 5, 7, 7, 11, 11, 11, 11, 13, 13];
        if n < FAST_BKT.len() {
            if n == 0 {
                return 1;
            }
            self.next_resize = FAST_BKT[n];
            return FAST_BKT[n];
        }
        let p = match STD_PRIMES.iter().find(|&&p| p >= n) {
            Some(&p) => p,
            // (Past the part of the list kept here: more elements than the
            // 0x3fffff ids a module can have.)
            None => {
                let mut p = n.max(n + n / 13);
                while !is_prime(p) {
                    p += 1;
                }
                p
            }
        };
        self.next_resize = p;
        p
    }

    // _Prime_rehash_policy::_M_need_rehash(), max load factor 1.
    fn need_rehash(&mut self, n_bkt: usize, n_elt: usize, n_ins: usize) -> Option<usize> {
        if n_elt + n_ins > self.next_resize {
            // If _M_next_resize is 0 it means that we have nothing allocated so far and that we start inserting
            // elements. In this case we start with an initial bucket size of 11.
            let min_bkts = (n_elt + n_ins).max(if self.next_resize != 0 { 0 } else { 11 });
            if min_bkts >= n_bkt {
                let n = (min_bkts + 1).max(n_bkt * 2);
                return Some(self.next_bkt(n));
            }
            self.next_resize = n_bkt;
        }
        None
    }

    #[inline]
    fn bucket_of(&self, key: u32) -> usize {
        key as usize % self.buckets.len()
    }

    fn next_of(&self, before: usize) -> usize {
        if before == BEFORE_BEGIN {
            self.head
        } else {
            self.nodes[before].next
        }
    }

    fn set_next(&mut self, before: usize, node: usize) {
        if before == BEFORE_BEGIN {
            self.head = node;
        } else {
            self.nodes[before].next = node;
        }
    }

    // _M_rehash_aux() for unique keys.
    fn rehash(&mut self, n: usize) {
        let mut new_buckets = vec![NIL; n];
        let mut p = self.head;
        self.head = NIL;
        let mut bbegin_bkt = 0;
        while p != NIL {
            let next = self.nodes[p].next;
            let bkt = self.nodes[p].key as usize % n;
            if new_buckets[bkt] == NIL {
                self.nodes[p].next = self.head;
                self.head = p;
                new_buckets[bkt] = BEFORE_BEGIN;
                if self.nodes[p].next != NIL {
                    new_buckets[bbegin_bkt] = p;
                }
                bbegin_bkt = bkt;
            } else {
                let before = new_buckets[bkt];
                let after = if before == BEFORE_BEGIN {
                    self.head
                } else {
                    self.nodes[before].next
                };
                self.nodes[p].next = after;
                if before == BEFORE_BEGIN {
                    self.head = p;
                } else {
                    self.nodes[before].next = p;
                }
            }
            p = next;
        }
        self.buckets = new_buckets;
    }

    // _M_insert_bucket_begin().
    fn insert_bucket_begin(&mut self, bkt: usize, node: usize) {
        if self.buckets[bkt] != NIL {
            // Bucket is not empty, we just need to insert the new node
            // after the bucket before begin.
            let before = self.buckets[bkt];
            let after = self.next_of(before);
            self.nodes[node].next = after;
            self.set_next(before, node);
        } else {
            // The bucket is empty, the new node is inserted at the
            // beginning of the singly-linked list and the bucket will
            // contain _M_before_begin pointer.
            self.nodes[node].next = self.head;
            self.head = node;
            let next = self.nodes[node].next;
            if next != NIL {
                // We must update former begin bucket that is pointing to
                // _M_before_begin.
                let b = self.bucket_of(self.nodes[next].key);
                self.buckets[b] = node;
            }
            self.buckets[bkt] = BEFORE_BEGIN;
        }
    }

    fn insert_new(&mut self, key: u32, value: T) -> usize {
        let n_bkt = self.buckets.len();
        let n_elt = self.index.len();
        if let Some(n) = self.need_rehash(n_bkt, n_elt, 1) {
            self.rehash(n);
        }
        let node = Node {
            key,
            value,
            next: NIL,
        };
        let idx = match self.free.pop() {
            Some(i) => {
                self.nodes[i] = node;
                i
            }
            None => {
                self.nodes.push(node);
                self.nodes.len() - 1
            }
        };
        let bkt = self.bucket_of(key);
        self.insert_bucket_begin(bkt, idx);
        self.index.insert(key, idx);
        idx
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn contains_key(&self, key: &u32) -> bool {
        self.index.contains_key(key)
    }

    pub fn get(&self, key: &u32) -> Option<&T> {
        self.index.get(key).map(|&i| &self.nodes[i].value)
    }

    pub fn get_mut(&mut self, key: &u32) -> Option<&mut T> {
        match self.index.get(key) {
            Some(&i) => Some(&mut self.nodes[i].value),
            None => None,
        }
    }

    /// `operator[]`.
    pub fn entry_or_default(&mut self, key: u32) -> &mut T
    where
        T: Default,
    {
        let idx = match self.index.get(&key) {
            Some(&i) => i,
            None => self.insert_new(key, T::default()),
        };
        &mut self.nodes[idx].value
    }

    /// `insert({key, value})`: keeps an existing value. Returns whether it
    /// inserted.
    pub fn insert_keep(&mut self, key: u32, value: T) -> bool {
        if self.index.contains_key(&key) {
            return false;
        }
        self.insert_new(key, value);
        true
    }

    /// `map[key] = value`.
    pub fn insert(&mut self, key: u32, value: T) {
        match self.index.get(&key) {
            Some(&i) => self.nodes[i].value = value,
            None => {
                self.insert_new(key, value);
            }
        }
    }

    /// `erase(key)`: _M_erase() unlinks the node, fixing the buckets.
    pub fn remove(&mut self, key: &u32) -> bool {
        let Some(idx) = self.index.remove(key) else {
            return false;
        };
        let bkt = self.bucket_of(self.nodes[idx].key);
        // Find the node before idx.
        let mut prev = self.buckets[bkt];
        while self.next_of(prev) != idx {
            prev = self.next_of(prev);
        }
        let next = self.nodes[idx].next;
        // _M_remove_bucket_begin() / the next bucket's before pointer.
        if prev == self.buckets[bkt] {
            // idx is the first node of its bucket.
            let next_bkt = if next != NIL {
                self.bucket_of(self.nodes[next].key)
            } else {
                NIL
            };
            if next == NIL || next_bkt != bkt {
                // Bucket is now empty.
                if next != NIL {
                    self.buckets[next_bkt] = self.buckets[bkt];
                }
                self.buckets[bkt] = NIL;
            }
        } else if next != NIL {
            let next_bkt = self.bucket_of(self.nodes[next].key);
            if next_bkt != bkt {
                self.buckets[next_bkt] = prev;
            }
        }
        self.set_next(prev, next);
        self.free.push(idx);
        true
    }

    pub fn clear(&mut self) {
        // clear() keeps the bucket count.
        self.nodes.clear();
        self.free.clear();
        self.head = NIL;
        for b in self.buckets.iter_mut() {
            *b = NIL;
        }
        self.index.clear();
    }

    /// The keys, in iteration order.
    pub fn keys(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.len());
        let mut p = self.head;
        while p != NIL {
            out.push(self.nodes[p].key);
            p = self.nodes[p].next;
        }
        out
    }

    /// The entries, in iteration order.
    pub fn iter(&self) -> StdHashMapIter<'_, T> {
        StdHashMapIter {
            map: self,
            p: self.head,
        }
    }
}

pub struct StdHashMapIter<'a, T> {
    map: &'a StdHashMap<T>,
    p: usize,
}

impl<'a, T> Iterator for StdHashMapIter<'a, T> {
    type Item = (u32, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        if self.p == NIL {
            return None;
        }
        let n = &self.map.nodes[self.p];
        self.p = n.next;
        Some((n.key, &n.value))
    }
}

/// `std::unordered_set<uint32_t>` in libstdc++'s iteration order.
#[derive(Clone, Debug, Default)]
pub struct StdHashSet {
    map: StdHashMap<()>,
}

impl StdHashSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: u32) -> bool {
        self.map.insert_keep(key, ())
    }

    pub fn contains(&self, key: &u32) -> bool {
        self.map.contains_key(key)
    }

    pub fn remove(&mut self, key: &u32) -> bool {
        self.map.remove(key)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear()
    }

    /// The elements, in iteration order.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.map.iter().map(|(k, _)| k)
    }

    pub fn to_vec(&self) -> Vec<u32> {
        self.map.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(s: &mut u32) -> u32 {
        *s = s.wrapping_mul(1103515245).wrapping_add(12345);
        *s >> 8
    }

    /// (size, FNV-1a over the keys in iteration order) for 64 pseudo-random
    /// insert/erase sequences, recorded from libstdc++'s
    /// `std::unordered_map<uint32_t, uint32_t>` (GCC 13).
    const EXPECTED: [(usize, u64); 64] = [
        (9, 0xa4ff843ec963fabf),
        (31, 0xc9640b5247fe372a),
        (16, 0x97f3a5d2a02bed47),
        (0, 0x14650fb0739d0383),
        (15, 0x5d75ad421660bfbf),
        (4, 0x85d1a4f9d95b8e54),
        (22, 0xf597060469577e42),
        (8, 0xef33ee258269a699),
        (24, 0x46b6cffb0d1190f6),
        (12, 0x28f0b46ec6aa1b24),
        (25, 0x95177d467d1f33cb),
        (15, 0xa6442c1152a69e03),
        (31, 0x911b6f2d2122ec11),
        (19, 0x37f10c01f7a5dc9f),
        (3, 0x992e8850e6726c61),
        (21, 0x603ed66c3832cdb1),
        (5, 0x95a2732fb3ea0598),
        (22, 0xe33a768de25acc24),
        (9, 0xc09210d78b21773c),
        (24, 0x8b2f87c1dee8bd96),
        (13, 0xdc75164e3415bb11),
        (30, 0x27d0278add0881e6),
        (17, 0xd873102d02ddd241),
        (1, 0x44b9bbd473c72049),
        (14, 0x89974cb491fe6b46),
        (5, 0x3cd20a01845b683e),
        (15, 0xb8c51f05b23920e7),
        (8, 0xf1e958824045846c),
        (21, 0xcea6ba4aaaefd3e2),
        (10, 0x9398d869fe1261fd),
        (24, 0x31e4cb213e49f405),
        (15, 0x7cf439cf972d5e07),
        (146, 0x85f0ad10ecdc4c85),
        (170, 0xd429ca9b09f3660a),
        (75, 0x89bba3e763a4ab54),
        (281, 0xe10238f184621f59),
        (56, 0xe4ded2ab15c4ac41),
        (222, 0xe0614163f46db132),
        (353, 0x29628cf0b35bd8bd),
        (175, 0x57c3ec7f17b8f3ae),
        (370, 0xfe27ccac0ec419c4),
        (140, 0x7ef172f64894bca5),
        (316, 0x5c5bc920b89b1999),
        (83, 0x7bc4b38343c6882e),
        (250, 0xfa0599438f841680),
        (22, 0xd7d0e3e6bf7f30a9),
        (161, 0xb0ebb5c80695a71e),
        (189, 0xf80879edced47ffd),
        (99, 0x228435b798d477f4),
        (27, 0xa3a5b35031bd527a),
        (93, 0x7356539a1662c282),
        (262, 0x86b95773e43d15e2),
        (45, 0xc4506436f54d661c),
        (203, 0xe339b2b71b8670d8),
        (297, 0x0b8c1774ad448709),
        (187, 0x098814d241ea5412),
        (362, 0xc55982582bb682a8),
        (113, 0xfeebdfd7ccab8b4b),
        (291, 0xf7a6fca8441845fb),
        (77, 0x374c63aeefb772ff),
        (242, 0x39c2bf02e296e4c0),
        (14, 0xb754050cba570b57),
        (100, 0xfc2a263df1605dff),
        (28, 0xc05ca591a8b4f2b9),
    ];

    #[test]
    fn iteration_order_matches_libstdcxx() {
        for (seq, &(size, hash)) in EXPECTED.iter().enumerate() {
            let seq = seq as u32;
            let mut s = seq * 977 + 1;
            let mut m = StdHashMap::<u32>::new();
            let n = 1 + lcg(&mut s) % if seq < 32 { 40 } else { 600 };
            let range = 1 + lcg(&mut s) % 2000;
            for i in 0..n {
                let k = lcg(&mut s) % range;
                if lcg(&mut s) % 10 < 8 {
                    *m.entry_or_default(k) = i;
                } else {
                    m.remove(&k);
                }
            }
            let mut h: u64 = 1469598103934665603;
            for (k, _) in m.iter() {
                h = (h ^ u64::from(k)).wrapping_mul(1099511628211);
            }
            assert_eq!((m.len(), h), (size, hash), "sequence {seq}");
        }
    }
}
