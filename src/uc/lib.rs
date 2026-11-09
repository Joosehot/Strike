//! What lists, slices and arrays need beyond single operations, written
//! in Strike IR: making them, growing them, copying them, printing them.
//! The lowering asks for a function by its name and only the functions a
//! program uses are made; they are branded, inlined and optimized as the
//! program's own functions are.
//!
//! A Python list is the address of three words, changed in place: its
//! length, its capacity, the address of its elements. A Go slice is the
//! address of three words never changed once made (the elements' address,
//! the length, the capacity), so a copy of a slice is a copy of its address,
//! as Go copies the three words. A C or Go array is the address of its
//! elements.

use super::ir::{BinOp, Func, Op, ALLOC};
use std::collections::BTreeMap;

/// Go's allocator's size classes (internal/runtime/gc/sizeclasses.go, Go
/// 1.27): a slice that grows gets the capacity its new memory's class
/// holds, and that capacity can be seen (cap, and which appends share
/// memory), so it is Go's, not a guess.
const GO_CLASSES: [i64; 67] = [
    8, 16, 24, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256, 288, 320, 352, 384, 416, 448, 480, 512, 576, 640, 704, 768, 896, 1024, 1152, 1280, 1408, 1536, 1792, 2048, 2304, 2688, 3072, 3200, 3456, 4096, 4864, 5376, 6144, 6528, 6784, 6912, 8192, 9472, 9728, 10240, 10880, 12288, 13568, 14336, 16384, 18432, 19072, 20480, 21760, 24576, 27264, 28672, 32768,
];

/// The most elements a list or slice may have: past it the program stops
/// as out of memory, never wrapping a size.
pub const MAX_LEN: i64 = 1 << 40;

pub const OUT_OF_MEMORY: &[u8] = b"strike: out of memory: a list or slice that large is not made, so the program stops\n";

/// A function being written: its code, its labels and its slots.
pub(super) struct B {
    pub(super) code: Vec<Op>,
    labels: u32,
}

impl B {
    pub(super) fn new() -> B {
        B { code: Vec::new(), labels: 0 }
    }
    pub(super) fn label(&mut self) -> u32 {
        self.labels += 1;
        self.labels - 1
    }
    pub(super) fn o(&mut self, ops: &[Op]) {
        self.code.extend_from_slice(ops);
    }
    /// The word at slot s plus off bytes.
    pub(super) fn field(&mut self, s: u32, off: i64) {
        self.code.push(Op::Load(s));
        if off != 0 {
            self.o(&[Op::Push(off), Op::Bin(BinOp::Add)]);
        }
        self.code.push(Op::LoadM(8));
    }
    /// Pushes the address of slot s plus off bytes.
    fn at(&mut self, s: u32, off: i64) {
        self.code.push(Op::Load(s));
        if off != 0 {
            self.o(&[Op::Push(off), Op::Bin(BinOp::Add)]);
        }
    }
    /// Stops the program as out of memory unless 0 <= slot n <= MAX_LEN.
    fn size_check(&mut self, n: u32) {
        let ok = self.label();
        self.o(&[Op::Load(n), Op::Push(MAX_LEN), Op::Bin(BinOp::Ule), Op::Jnz(ok), Op::Fail(1, OUT_OF_MEMORY.to_vec()), Op::Label(ok)]);
    }
    pub(super) fn done(self, name: &str, params: u32, slots: u32) -> Func {
        Func::new(name, params, slots, self.code)
    }
}

/// Every library function the program asked for, by name; each brings the
/// ones it calls.
pub fn make(printers: &BTreeMap<String, Printer>, basics: &std::collections::HashSet<String>) -> Vec<Func> {
    let mut out: BTreeMap<String, Func> = BTreeMap::new();
    let mut todo: Vec<String> = printers.keys().chain(basics.iter()).cloned().collect();
    while let Some(name) = todo.pop() {
        if out.contains_key(&name) {
            continue;
        }
        let f = match printers.get(&name) {
            Some(p) => print(&name, p),
            None => basic(&name).unwrap_or_else(|| panic!("no library function {name}")),
        };
        for op in &f.code {
            if let Op::Call(c, _) = op {
                if c.starts_with("lib.") && !out.contains_key(c) {
                    todo.push(c.clone());
                }
            }
        }
        out.insert(name, f);
    }
    out.into_values().collect()
}

pub(super) fn call(name: &str, n: u32) -> Op {
    Op::Call(name.to_string(), n)
}

fn alloc() -> Op {
    Op::Call(ALLOC.to_string(), 1)
}

fn basic(name: &str) -> Option<Func> {
    use BinOp::*;
    let mut b = B::new();
    Some(match name {
        // (dst, src, n): n bytes from src to dst, 8 at a time, then the rest
        "lib.copy" => {
            let (top, end, top1, end1) = (b.label(), b.label(), b.label(), b.label());
            b.o(&[Op::Label(top), Op::Load(3), Op::Push(8), Op::Bin(Add), Op::Load(2), Op::Bin(Le), Op::Jz(end)]);
            b.o(&[Op::Load(0), Op::Load(3), Op::Bin(Add), Op::Load(1), Op::Load(3), Op::Bin(Add), Op::LoadM(8), Op::StoreM(8)]);
            b.o(&[Op::Load(3), Op::Push(8), Op::Bin(Add), Op::Store(3), Op::Jmp(top), Op::Label(end)]);
            b.o(&[Op::Label(top1), Op::Load(3), Op::Load(2), Op::Bin(Lt), Op::Jz(end1)]);
            b.o(&[Op::Load(0), Op::Load(3), Op::Bin(Add), Op::Load(1), Op::Load(3), Op::Bin(Add), Op::LoadM(1), Op::StoreM(1)]);
            b.o(&[Op::Load(3), Op::Push(1), Op::Bin(Add), Op::Store(3), Op::Jmp(top1), Op::Label(end1), Op::Push(0), Op::Ret]);
            b.done(name, 3, 4)
        }
        // (n bytes): new zero memory, the size checked
        "lib.bytes" => {
            let ok = b.label();
            b.o(&[Op::Load(0), Op::Push(MAX_LEN * 8), Op::Bin(Ule), Op::Jnz(ok), Op::Fail(1, OUT_OF_MEMORY.to_vec()), Op::Label(ok)]);
            b.o(&[Op::Load(0), alloc(), Op::Ret]);
            b.done(name, 1, 1)
        }
        // (dst, n, v): n words set to v
        "lib.fill" => {
            let (top, end) = (b.label(), b.label());
            b.o(&[Op::Label(top), Op::Load(3), Op::Load(1), Op::Bin(Lt), Op::Jz(end)]);
            b.o(&[Op::Load(0), Op::Load(3), Op::Push(3), Op::Bin(Shl), Op::Bin(Add), Op::Load(2), Op::StoreM(8)]);
            b.o(&[Op::Load(3), Op::Push(1), Op::Bin(Add), Op::Store(3), Op::Jmp(top), Op::Label(end), Op::Push(0), Op::Ret]);
            b.done(name, 3, 4)
        }
        // (n words): new zero memory, the size checked
        "lib.words" => {
            b.size_check(0);
            b.o(&[Op::Load(0), Op::Push(3), Op::Bin(Shl), alloc(), Op::Ret]);
            b.done(name, 1, 1)
        }
        // (n): a list of n zeros
        "lib.list_new" => {
            b.o(&[Op::Push(24), alloc(), Op::Store(1)]);
            b.o(&[Op::Load(1), Op::Load(0), Op::StoreM(8)]);
            b.at(1, 8);
            b.o(&[Op::Load(0), Op::StoreM(8)]);
            b.at(1, 16);
            b.o(&[Op::Load(0), call("lib.words", 1), Op::StoreM(8), Op::Load(1), Op::Ret]);
            b.done(name, 1, 2)
        }
        // (list, x): x at the end, the elements moved to twice the room
        // when they fill it
        "lib.list_append" => {
            let have = b.label();
            b.field(0, 0);
            b.o(&[Op::Store(2)]);
            b.o(&[Op::Load(2)]);
            b.field(0, 8);
            b.o(&[Op::Bin(Eq), Op::Jz(have)]);
            b.o(&[Op::Load(2), Op::Push(1), Op::Bin(Shl), Op::Push(4), Op::Bin(Add), Op::Store(4)]);
            b.o(&[Op::Load(4), call("lib.words", 1), Op::Store(3)]);
            b.o(&[Op::Load(3)]);
            b.field(0, 16);
            b.o(&[Op::Load(2), Op::Push(3), Op::Bin(Shl), call("lib.copy", 3), Op::Pop]);
            b.at(0, 16);
            b.o(&[Op::Load(3), Op::StoreM(8)]);
            b.at(0, 8);
            b.o(&[Op::Load(4), Op::StoreM(8), Op::Label(have)]);
            b.field(0, 16);
            b.o(&[Op::Load(2), Op::Push(3), Op::Bin(Shl), Op::Bin(Add), Op::Load(1), Op::StoreM(8)]);
            b.o(&[Op::Load(0), Op::Load(2), Op::Push(1), Op::Bin(Add), Op::StoreM(8), Op::Push(0), Op::Ret]);
            b.done(name, 2, 5)
        }
        // (list, other): other's elements appended (Python's +=, extend)
        "lib.list_extend" => {
            let (top, end) = (b.label(), b.label());
            b.field(1, 0);
            b.o(&[Op::Store(3)]);
            b.o(&[Op::Label(top), Op::Load(2), Op::Load(3), Op::Bin(Lt), Op::Jz(end), Op::Load(0)]);
            b.field(1, 16);
            b.o(&[Op::Load(2), Op::Push(3), Op::Bin(Shl), Op::Bin(Add), Op::LoadM(8), call("lib.list_append", 2), Op::Pop]);
            b.o(&[Op::Load(2), Op::Push(1), Op::Bin(Add), Op::Store(2), Op::Jmp(top), Op::Label(end), Op::Push(0), Op::Ret]);
            b.done(name, 2, 4)
        }
        // (list): the last element, taken off
        "lib.list_pop" => {
            let ok = b.label();
            b.field(0, 0);
            b.o(&[Op::Push(1), Op::Bin(Sub), Op::Store(1)]);
            b.o(&[Op::Load(1), Op::Push(0), Op::Bin(Ge), Op::Jnz(ok), Op::Fail(1, b"IndexError: pop from empty list\n".to_vec()), Op::Label(ok)]);
            b.o(&[Op::Load(0), Op::Load(1), Op::StoreM(8)]);
            b.field(0, 16);
            b.o(&[Op::Load(1), Op::Push(3), Op::Bin(Shl), Op::Bin(Add), Op::LoadM(8), Op::Ret]);
            b.done(name, 1, 2)
        }
        // (list, n): a new list, the elements n times (none for n <= 0)
        "lib.list_repeat" => {
            let (pos, top, end) = (b.label(), b.label(), b.label());
            b.o(&[Op::Load(1), Op::Push(0), Op::Bin(Lt), Op::Jz(pos), Op::Push(0), Op::Store(1), Op::Label(pos)]);
            b.field(0, 0);
            b.o(&[Op::Store(2), Op::Load(2), Op::Load(1), Op::Bin(MulOv), Op::Store(5)]);
            b.o(&[Op::Load(5), call("lib.list_new", 1), Op::Store(3)]);
            b.o(&[Op::Label(top), Op::Load(4), Op::Load(1), Op::Bin(Lt), Op::Jz(end)]);
            b.field(3, 16);
            b.o(&[Op::Load(4), Op::Load(2), Op::Bin(Mul), Op::Push(3), Op::Bin(Shl), Op::Bin(Add)]);
            b.field(0, 16);
            b.o(&[Op::Load(2), Op::Push(3), Op::Bin(Shl), call("lib.copy", 3), Op::Pop]);
            b.o(&[Op::Load(4), Op::Push(1), Op::Bin(Add), Op::Store(4), Op::Jmp(top), Op::Label(end), Op::Load(3), Op::Ret]);
            b.done(name, 2, 6)
        }
        // (a, b): a new list, a's elements then b's
        "lib.list_concat" => {
            b.field(0, 0);
            b.o(&[Op::Store(3), Op::Load(3)]);
            b.field(1, 0);
            b.o(&[Op::Bin(AddOv), call("lib.list_new", 1), Op::Store(2)]);
            b.field(2, 16);
            b.field(0, 16);
            b.o(&[Op::Load(3), Op::Push(3), Op::Bin(Shl), call("lib.copy", 3), Op::Pop]);
            b.field(2, 16);
            b.o(&[Op::Load(3), Op::Push(3), Op::Bin(Shl), Op::Bin(Add)]);
            b.field(1, 16);
            b.field(1, 0);
            b.o(&[Op::Push(3), Op::Bin(Shl), call("lib.copy", 3), Op::Pop, Op::Load(2), Op::Ret]);
            b.done(name, 2, 4)
        }
        // (list, lo, hi): Python's list[lo:hi], each side i64::MIN where
        // it is left out: negative from the end, then held to 0..len
        "lib.list_slice" => {
            let len = 3;
            b.field(0, 0);
            b.o(&[Op::Store(len)]);
            for (s, default) in [(1u32, None), (2u32, Some(len))] {
                let (given, neg, low, high, done) = (b.label(), b.label(), b.label(), b.label(), b.label());
                b.o(&[Op::Load(s), Op::Push(i64::MIN), Op::Bin(Ne), Op::Jnz(given)]);
                match default {
                    None => b.o(&[Op::Push(0), Op::Store(s)]),
                    Some(l) => b.o(&[Op::Load(l), Op::Store(s)]),
                }
                b.o(&[Op::Jmp(done), Op::Label(given)]);
                b.o(&[Op::Load(s), Op::Push(0), Op::Bin(Lt), Op::Jz(neg), Op::Load(s), Op::Load(len), Op::Bin(Add), Op::Store(s), Op::Label(neg)]);
                b.o(&[Op::Load(s), Op::Push(0), Op::Bin(Lt), Op::Jz(low), Op::Push(0), Op::Store(s), Op::Label(low)]);
                b.o(&[Op::Load(s), Op::Load(len), Op::Bin(Gt), Op::Jz(high), Op::Load(len), Op::Store(s), Op::Label(high), Op::Label(done)]);
            }
            let ok = b.label();
            b.o(&[Op::Load(2), Op::Load(1), Op::Bin(Lt), Op::Jz(ok), Op::Load(1), Op::Store(2), Op::Label(ok)]);
            b.o(&[Op::Load(2), Op::Load(1), Op::Bin(Sub), Op::Store(4), Op::Load(4), call("lib.list_new", 1), Op::Store(5)]);
            b.field(5, 16);
            b.field(0, 16);
            b.o(&[Op::Load(1), Op::Push(3), Op::Bin(Shl), Op::Bin(Add), Op::Load(4), Op::Push(3), Op::Bin(Shl), call("lib.copy", 3), Op::Pop, Op::Load(5), Op::Ret]);
            b.done(name, 3, 6)
        }
        // (data, len, cap): a slice's three words
        "lib.header" => {
            b.o(&[Op::Push(24), alloc(), Op::Store(3)]);
            b.o(&[Op::Load(3), Op::Load(0), Op::StoreM(8)]);
            b.at(3, 8);
            b.o(&[Op::Load(1), Op::StoreM(8)]);
            b.at(3, 16);
            b.o(&[Op::Load(2), Op::StoreM(8), Op::Load(3), Op::Ret]);
            b.done(name, 3, 4)
        }
        // (len, cap, bytes per element): make([]T, len, cap), checked by the caller
        "lib.slice_new" => {
            b.o(&[Op::Load(1), Op::Load(2), Op::Bin(Mul), call("lib.bytes", 1), Op::Load(0), Op::Load(1), call("lib.header", 3), Op::Ret]);
            b.done(name, 3, 3)
        }
        // (newLen, oldCap): Go's runtime.nextslicecap
        "lib.go_nextcap" => {
            let (a, c, again) = (b.label(), b.label(), b.label());
            let d = b.label();
            b.o(&[Op::Load(1), Op::Store(2)]);
            b.o(&[Op::Load(0), Op::Load(1), Op::Push(1), Op::Bin(Shl), Op::Bin(Gt), Op::Jz(a), Op::Load(0), Op::Ret, Op::Label(a)]);
            b.o(&[Op::Load(1), Op::Push(256), Op::Bin(Lt), Op::Jz(c), Op::Load(1), Op::Push(1), Op::Bin(Shl), Op::Ret, Op::Label(c)]);
            b.o(&[Op::Label(again), Op::Load(2), Op::Load(2), Op::Push(768), Op::Bin(Add), Op::Push(2), Op::Bin(Shr), Op::Bin(Add), Op::Store(2)]);
            b.o(&[Op::Load(2), Op::Load(0), Op::Bin(Ult), Op::Jnz(again)]);
            b.o(&[Op::Load(2), Op::Push(0), Op::Bin(Le), Op::Jz(d), Op::Load(0), Op::Ret, Op::Label(d), Op::Load(2), Op::Ret]);
            b.done(name, 2, 3)
        }
        // (size, noscan): Go's runtime.roundupsize
        "lib.go_roundup" => {
            let (large, small, ok) = (b.label(), b.label(), b.label());
            b.o(&[Op::Load(0), Op::Store(2), Op::Load(0), Op::Push(32768 - 8), Op::Bin(Le), Op::Jz(large)]);
            b.o(&[Op::Load(1), Op::Jnz(small), Op::Load(0), Op::Push(512), Op::Bin(Gt), Op::Jz(small)]);
            b.o(&[Op::Load(0), Op::Push(8), Op::Bin(Add), Op::Store(2), Op::Label(small)]);
            for c in GO_CLASSES {
                let next = b.label();
                b.o(&[Op::Load(2), Op::Push(c), Op::Bin(Le), Op::Jz(next), Op::Push(c), Op::Load(2), Op::Load(0), Op::Bin(Sub), Op::Bin(Sub), Op::Ret, Op::Label(next)]);
            }
            b.o(&[Op::Load(0), Op::Ret, Op::Label(large)]);
            b.o(&[Op::Load(0), Op::Push(8191), Op::Bin(Add), Op::Store(2), Op::Load(2), Op::Load(0), Op::Bin(Ult), Op::Jz(ok), Op::Load(0), Op::Ret, Op::Label(ok)]);
            b.o(&[Op::Load(2), Op::Push(!8191), Op::Bin(Band), Op::Ret]);
            b.done(name, 2, 3)
        }
        // (slice, k, Go's element size, noscan, bytes per element): a new
        // header k elements longer, the elements moved where they do not fit
        "lib.slice_grow" => {
            let grow = b.label();
            let (old, cap, nl, nc, nd) = (5, 6, 7, 8, 9);
            b.field(0, 8);
            b.o(&[Op::Store(old)]);
            b.field(0, 16);
            b.o(&[Op::Store(cap), Op::Load(old), Op::Load(1), Op::Bin(Add), Op::Store(nl)]);
            b.size_check(nl);
            b.o(&[Op::Load(nl), Op::Load(cap), Op::Bin(Le), Op::Jz(grow)]);
            b.field(0, 0);
            b.o(&[Op::Load(nl), Op::Load(cap), call("lib.header", 3), Op::Ret, Op::Label(grow)]);
            b.o(&[Op::Load(nl), Op::Load(cap), call("lib.go_nextcap", 2), Op::Store(nc)]);
            b.o(&[Op::Load(nc), Op::Load(2), Op::Bin(Mul), Op::Load(3), call("lib.go_roundup", 2), Op::Load(2), Op::Bin(DivTrunc), Op::Store(nc)]);
            b.o(&[Op::Load(nc), Op::Load(4), Op::Bin(Mul), call("lib.bytes", 1), Op::Store(nd)]);
            b.o(&[Op::Load(nd)]);
            b.field(0, 0);
            b.o(&[Op::Load(old), Op::Load(4), Op::Bin(Mul), call("lib.copy", 3), Op::Pop]);
            b.o(&[Op::Load(nd), Op::Load(nl), Op::Load(nc), call("lib.header", 3), Op::Ret]);
            b.done(name, 5, 10)
        }
        _ => return super::strs::make(name),
    })
}

/// How the elements of a list print: their kind, and the language's words.
#[derive(Clone, Debug, PartialEq)]
pub struct Printer {
    /// the container: a list or slice by its header, or an array of n
    pub array: Option<u64>,
    /// a Python list's header (length first), else a Go slice's (data first)
    pub list: bool,
    pub elem: Elem,
    /// bytes per element, and the bytes a number element is read with
    pub stride: u64,
    pub width: u8,
    pub open: Vec<u8>,
    pub sep: Vec<u8>,
    pub close: Vec<u8>,
    pub words: Option<(Vec<u8>, Vec<u8>)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Elem {
    Int,
    Bool,
    /// a list or slice: the element's own printer; true where the element
    /// is held in place (an array), not by its address
    Nested(String, bool),
    /// a string: true where it is shown as Python's repr, false as it is
    Str(bool),
}

fn print(name: &str, p: &Printer) -> Func {
    use BinOp::*;
    // slots: 0 the container, 1 i, 2 n, 3 data
    let mut b = B::new();
    let (top, end, first) = (b.label(), b.label(), b.label());
    match (p.array, p.list) {
        (Some(n), _) => b.o(&[Op::Push(n as i64), Op::Store(2), Op::Load(0), Op::Store(3)]),
        (None, true) => {
            b.field(0, 0);
            b.o(&[Op::Store(2)]);
            b.field(0, 16);
            b.o(&[Op::Store(3)]);
        }
        (None, false) => {
            b.field(0, 8);
            b.o(&[Op::Store(2)]);
            b.field(0, 0);
            b.o(&[Op::Store(3)]);
        }
    }
    b.o(&[Op::PrintStr(p.open.clone())]);
    b.o(&[Op::Label(top), Op::Load(1), Op::Load(2), Op::Bin(Lt), Op::Jz(end)]);
    b.o(&[Op::Load(1), Op::Jz(first), Op::PrintStr(p.sep.clone()), Op::Label(first)]);
    b.o(&[Op::Load(3), Op::Load(1), Op::Push(p.stride as i64), Op::Bin(Mul), Op::Bin(Add)]);
    match &p.elem {
        Elem::Nested(f, inline) => {
            if !inline {
                b.o(&[Op::LoadM(8)]);
            }
            b.o(&[call(f, 1), Op::Pop]);
        }
        Elem::Int => b.o(&[Op::LoadM(p.width), Op::PrintInt]),
        Elem::Str(repr) => b.o(&[Op::LoadM(8), call(if *repr { "lib.py_repr_print" } else { "lib.str_print" }, 1), Op::Pop]),
        Elem::Bool => {
            let (yes, no) = p.words.clone().unwrap_or((b"1".to_vec(), b"0".to_vec()));
            let (other, done) = (b.label(), b.label());
            b.o(&[Op::LoadM(p.width), Op::Jz(other), Op::PrintStr(yes), Op::Jmp(done), Op::Label(other), Op::PrintStr(no), Op::Label(done)]);
        }
    }
    b.o(&[Op::Load(1), Op::Push(1), Op::Bin(Add), Op::Store(1), Op::Jmp(top), Op::Label(end), Op::PrintStr(p.close.clone()), Op::Push(0), Op::Ret]);
    b.done(name, 1, 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Go's growth as runtime.growslice works it out, for elements of
    /// `size` bytes: the capacity after appending to a slice of `cap`.
    fn go_cap(new_len: i64, cap: i64, size: i64, noscan: bool) -> i64 {
        let mut nc = cap;
        let dbl = cap * 2;
        if new_len > dbl {
            nc = new_len;
        } else if cap < 256 {
            nc = dbl;
        } else {
            while nc < new_len {
                nc += (nc + 768) >> 2;
            }
        }
        let mut req = nc * size;
        let size0 = req;
        if req <= 32768 - 8 {
            if !noscan && req > 512 {
                req += 8;
            }
            let c = *GO_CLASSES.iter().find(|&&c| c >= req).unwrap();
            (c - (req - size0)) / size
        } else {
            ((req + 8191) & !8191) / size
        }
    }

    #[test]
    fn go_capacities_grow_as_go_grows_them() {
        // what Go 1.27 prints for cap() after appending one at a time to a
        // nil []int: 1 2 4 8 ... and past 256 the 1.25 rule
        let mut cap = 0;
        let mut seen = Vec::new();
        for len in 1..=2000 {
            if len > cap {
                cap = go_cap(len, cap, 8, true);
                seen.push(cap);
            }
        }
        assert_eq!(&seen[..10], &[1, 2, 4, 8, 16, 32, 64, 128, 256, 512]);
        assert_eq!(seen[10], 848);
        // []int32: 4-byte elements round to 8-byte classes
        assert_eq!(go_cap(1, 0, 4, true), 2);
        assert_eq!(go_cap(3, 2, 4, true), 4);
    }
}
