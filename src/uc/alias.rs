//! --release: which memory an operation can touch, exactly, from the
//! program's own addresses (no rule of C's about types is assumed).
//!
//! An address made of a global's address (AddrG) or the frame's (Frame),
//! plus whole numbers whose intervals ranges.rs knows, lies in a known run
//! of bytes of the globals or of the function's own memory: the globals
//! are one block of the zeros (global g at byte 8g) and the frame is the
//! function's own. Two such runs that do not overlap cannot touch the same
//! byte, whatever the program's arrays are; a global and the frame never
//! meet. Any other address (a parameter, a value read from memory, a heap
//! block) may be anywhere, and so may a call of the program's own
//! functions. A local read in an address is followed back to its store in
//! the same straight code (no label between), so a lowered `a[i] += x`,
//! whose address waits in a local, is known too.
//!
//! The API, for LICM and any later pass:
//! - `Facts::new(f, globals)` once for a function's code as it stands;
//! - `Facts::reads(i)` / `Facts::writes(i)`: what operation i reads or
//!   writes (None: nothing of the program's memory);
//! - `Facts::loop_writes(t, j)`: every write of the operations t..=j;
//! - `may_alias(a, b)`, and `Facts::in_bounds(m)`: a load of m cannot fault,
//!   so it may be done where the program would not have done it.

use super::ir::{BinOp, Func, Op, ALLOC, GETC, PRINT};
use super::ranges::{sites, Sites};
use super::simplify::{tree, E};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Base {
    /// the program's globals, from byte 0 (global g at 8g)
    Globals,
    /// the function's own memory (Op::Frame's bytes)
    Frame,
}

/// Memory an operation touches: the bytes lo..hi of a base, or anywhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mem {
    At(Base, i64, i64),
    Anywhere,
}

/// Can two accesses touch the same byte?
pub(super) fn may_alias(a: Mem, b: Mem) -> bool {
    match (a, b) {
        (Mem::At(x, alo, ahi), Mem::At(y, blo, bhi)) => x == y && alo < bhi && blo < ahi,
        _ => true,
    }
}

pub(super) struct Facts<'a> {
    f: &'a Func,
    st: Vec<Option<usize>>,
    sites: Sites,
    globals: i64,
}

/// How far a local in an address is followed back through stores.
const DEPTH: u32 = 4;
/// Addresses past this many bytes from their base are not taken as known.
const FAR: i64 = 1 << 40;

impl<'a> Facts<'a> {
    pub(super) fn new(f: &'a Func, globals: u32) -> Facts<'a> {
        // where every operand begins (an element read inside a value too)
        Facts { f, st: super::opt::starts_with(&f.code, &|_| true), sites: sites(f), globals: 8 * globals as i64 }
    }

    /// The bytes of a load or store of w bytes at the address code[s..=e],
    /// its locals read at operation k.
    fn address(&self, s: Option<usize>, e: usize, w: u8, k: usize) -> Mem {
        let Some(s) = s else { return Mem::Anywhere };
        let Some(t) = tree(&self.f.code[s..=e]) else { return Mem::Anywhere };
        match self.region(&t, k, 0) {
            Some((b, lo, hi)) => Mem::At(b, lo, hi + w as i64),
            None => Mem::Anywhere,
        }
    }

    /// (base, lowest, highest address) of an address expression.
    fn region(&self, e: &E, k: usize, depth: u32) -> Option<(Base, i64, i64)> {
        let known = |lo: i128, hi: i128| (lo > -(FAR as i128) && hi < FAR as i128).then_some((lo as i64, hi as i64));
        let int = |x: &E| {
            let r = self.sites.at(k, x);
            known(r.lo as i128, r.hi as i128)
        };
        match e {
            E::AddrG(g) => Some((Base::Globals, 8 * *g as i64, 8 * *g as i64)),
            E::Frame(n) => Some((Base::Frame, *n as i64, *n as i64)),
            E::Bin(BinOp::Add, a, b) => {
                let (base, lo, hi, r) = match self.region(a, k, depth) {
                    Some((base, lo, hi)) => (base, lo, hi, int(b)?),
                    None => {
                        let (base, lo, hi) = self.region(b, k, depth)?;
                        (base, lo, hi, int(a)?)
                    }
                };
                let (lo, hi) = known(lo as i128 + r.0 as i128, hi as i128 + r.1 as i128)?;
                Some((base, lo, hi))
            }
            E::Bin(BinOp::Sub, a, b) => {
                let (base, lo, hi) = self.region(a, k, depth)?;
                let r = int(b)?;
                let (lo, hi) = known(lo as i128 - r.1 as i128, hi as i128 - r.0 as i128)?;
                Some((base, lo, hi))
            }
            // a local holding an address: what its last store in the same
            // straight code stored, read where that store was
            E::Load(x) if depth < DEPTH => {
                let code = &self.f.code;
                let mut p = k;
                while p > 0 {
                    p -= 1;
                    match &code[p] {
                        Op::Label(_) => return None,
                        Op::Store(y) if y == x => {
                            let s = self.st.get(p.checked_sub(1)?).copied().flatten()?;
                            let v = tree(&code[s..p])?;
                            return self.region(&v, p, depth + 1);
                        }
                        _ => {}
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// The operand run of operation i that is its address (the value under
    /// the top for a store): its start, and its last operation.
    fn operand(&self, i: usize, under_value: bool) -> (Option<usize>, usize) {
        let top = i - 1;
        if !under_value {
            return (self.st[top], top);
        }
        match self.st[top] {
            Some(v) if v > 0 => (self.st[v - 1], v - 1),
            _ => (None, 0),
        }
    }

    /// What operation i reads of the program's memory, its address read
    /// where the operation is.
    #[allow(dead_code)] // the API for later passes (a vectorizer)
    pub(super) fn reads(&self, i: usize) -> Option<Mem> {
        self.reads_at(i, i)
    }

    /// The same, the address's locals read at operation k instead (where a
    /// load moved out of a loop will be done).
    pub(super) fn reads_at(&self, i: usize, k: usize) -> Option<Mem> {
        Some(match &self.f.code[i] {
            Op::LoadM(w) => {
                let (s, e) = self.operand(i, false);
                self.address(s, e, *w, k)
            }
            Op::LoadG(g) => Mem::At(Base::Globals, 8 * *g as i64, 8 * *g as i64 + 8),
            // a vector's 16 bytes (vectorize.rs)
            Op::VLoad(..) => {
                let (s, e) = self.operand(i, false);
                self.address(s, e, 16, k)
            }
            Op::OpM(_, w) => {
                let (s, e) = self.operand(i, true);
                self.address(s, e, *w, k)
            }
            Op::Call(name, _) if name == ALLOC || name == PRINT || name == GETC => return None,
            Op::Call(..) | Op::PrintMem | Op::FailMem(_) => Mem::Anywhere,
            _ => return None,
        })
    }

    /// What operation i writes of the program's memory.
    pub(super) fn writes(&self, i: usize) -> Option<Mem> {
        Some(match &self.f.code[i] {
            Op::StoreM(w) | Op::OpM(_, w) => {
                let (s, e) = self.operand(i, true);
                self.address(s, e, *w, i)
            }
            Op::StoreG(g) => Mem::At(Base::Globals, 8 * *g as i64, 8 * *g as i64 + 8),
            Op::VStore(..) => {
                let (s, e) = self.operand(i, false);
                self.address(s, e, 16, i)
            }
            // the runtime's own calls write none of the program's memory
            Op::Call(name, _) if name == ALLOC || name == PRINT || name == GETC => return None,
            Op::Call(..) => Mem::Anywhere,
            _ => return None,
        })
    }

    /// Every write of the operations t..=j.
    pub(super) fn loop_writes(&self, t: usize, j: usize) -> Vec<Mem> {
        (t..=j).filter_map(|i| self.writes(i)).collect()
    }

    /// Memory that is there to read: inside the globals or the frame (a load
    /// of it cannot fault).
    pub(super) fn in_bounds(&self, m: Mem) -> bool {
        match m {
            Mem::At(Base::Globals, lo, hi) => lo >= 0 && hi <= self.globals,
            Mem::At(Base::Frame, lo, hi) => lo >= 0 && hi <= self.f.frame as i64,
            Mem::Anywhere => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// for i in 0..5 { for j in i+1..5 { g[15 + j] = g[i] } }: the load of
    /// global i (globals 0..4) cannot meet the stores to globals 15..19.
    #[test]
    fn distinct_runs_of_the_globals_do_not_meet() {
        use BinOp::*;
        let code = vec![
            Op::Push(0),
            Op::Store(0),
            Op::Label(0),
            Op::Load(0),
            Op::Push(5),
            Op::Bin(Lt),
            Op::Jz(1),
            Op::Load(0),
            Op::Push(1),
            Op::Bin(Add),
            Op::Store(1),
            Op::Label(2),
            Op::Load(1),
            Op::Push(5),
            Op::Bin(Lt),
            Op::Jz(3),
            // g[15 + j] = g[i]
            Op::AddrG(15),
            Op::Load(1),
            Op::Push(3),
            Op::Bin(Shl),
            Op::Bin(Add),
            Op::AddrG(0),
            Op::Load(0),
            Op::Push(3),
            Op::Bin(Shl),
            Op::Bin(Add),
            Op::LoadM(8),
            Op::StoreM(8),
            Op::Load(1),
            Op::Push(1),
            Op::Bin(Add),
            Op::Store(1),
            Op::Jmp(2),
            Op::Label(3),
            Op::Load(0),
            Op::Push(1),
            Op::Bin(Add),
            Op::Store(0),
            Op::Jmp(0),
            Op::Label(1),
            Op::Push(0),
            Op::Ret,
        ];
        let f = Func::new("t", 0, 2, code);
        let facts = Facts::new(&f, 20);
        let load = facts.reads(26).unwrap();
        let store = facts.writes(27).unwrap();
        assert_eq!(load, Mem::At(Base::Globals, 0, 40));
        // j starts at 0 before its first store, so 15 + 0 counts too
        assert_eq!(store, Mem::At(Base::Globals, 120, 160));
        assert!(!may_alias(load, store));
        assert!(facts.in_bounds(load));
        // a store through a value read from memory may be anywhere
        assert!(may_alias(load, Mem::Anywhere));
        assert!(may_alias(Mem::At(Base::Globals, 0, 8), Mem::At(Base::Globals, 7, 9)));
        assert!(!may_alias(Mem::At(Base::Globals, 0, 8), Mem::At(Base::Frame, 0, 8)));
    }
}
