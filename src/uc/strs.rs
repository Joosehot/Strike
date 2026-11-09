//! Strings, written in Strike IR as the rest of the library is (lib.rs).
//!
//! A Python or Go string is the address of its length (8 bytes) and then
//! its bytes; it is never changed once made, so a copy of a string is a
//! copy of its address, and a literal lives in the read-only data
//! (Op::Data). A Go string is bytes; a Python string is code points, and
//! Strike holds a Python string to ASCII, so its bytes are its code points
//! (a literal past ASCII is refused, and a line read that holds one stops
//! the program). A C string is a char array or the address of chars, ended
//! by a zero byte.

use super::ir::{BinOp, Func, Op, UnOp, GETC, PRINT};
use super::lib::{call, B};

use BinOp::*;

pub const EOF_ERROR: &[u8] = b"EOFError: EOF when reading a line\n";
const NOT_ASCII_IN: &[u8] = b"strike: the line read holds a character past ASCII; Strike's Python strings hold ASCII only, so the program stops\n";
const LONE_CR: &[u8] = b"strike: the line read holds a carriage return that does not end it; Strike does not read such a line yet, so the program stops\n";
const NOT_AN_INT: &[u8] = b"ValueError: invalid literal for int() with base 10\n";
const CHR_PAST_ASCII: &[u8] = b"strike: chr() of a code point past ASCII; Strike's Python strings hold ASCII only, so the program stops\n";
const CHR_RANGE: &[u8] = b"ValueError: chr() arg not in range(0x110000)\n";
const ORD_ONE: &[u8] = b"TypeError: ord() expected a character\n";
const STR_INDEX: &[u8] = b"IndexError: string index out of range\n";
const QUOTE_PAST_ASCII: &[u8] = b"strike: %q of a string past ASCII is not in Strike yet (which characters Go prints as they are is Unicode's table), so the program stops\n";

/// The empty string.
pub fn empty() -> Op {
    Op::Data(vec![0; 8])
}

/// A string literal's bytes as they are kept: the length, then the bytes.
pub fn literal(bytes: &[u8]) -> Op {
    let mut d = (bytes.len() as u64).to_le_bytes().to_vec();
    d.extend_from_slice(bytes);
    Op::Data(d)
}

/// Every one-character ASCII string, 16 bytes each: a Python s[i] is one
/// of them, never new memory.
fn chars() -> Op {
    let mut d = Vec::with_capacity(128 * 16);
    for c in 0..128u8 {
        d.extend_from_slice(&1u64.to_le_bytes());
        d.push(c);
        d.extend_from_slice(&[0; 7]);
    }
    Op::Data(d)
}

impl B {
    /// Pushes the byte at string slot s, index slot i, as 0..255.
    fn sbyte(&mut self, s: u32, i: u32) {
        self.o(&[Op::Load(s), Op::Push(8), Op::Bin(Add), Op::Load(i), Op::Bin(Add), Op::LoadM(1), Op::Push(255), Op::Bin(Band)]);
    }
    /// Pushes the address of byte slot i of string slot s.
    fn sat(&mut self, s: u32, i: u32) {
        self.o(&[Op::Load(s), Op::Push(8), Op::Bin(Add), Op::Load(i), Op::Bin(Add)]);
    }
    /// The byte the ops push, written at byte slot n of string slot buf,
    /// and n one further.
    fn put(&mut self, buf: u32, n: u32, v: &[Op]) {
        self.sat(buf, n);
        self.o(v);
        self.o(&[Op::StoreM(1), Op::Load(n), Op::Push(1), Op::Bin(Add), Op::Store(n)]);
    }
    /// Pushes 1 if the byte in slot c is what Python's str.isspace() says
    /// is a blank (ASCII: \t \n \v \f \r, \x1c to \x1f, space), else 0.
    fn blank(&mut self, c: u32) {
        self.o(&[Op::Load(c), Op::Push(9), Op::Bin(Sub), Op::Push(5), Op::Bin(Ult), Op::Load(c), Op::Push(28), Op::Bin(Sub), Op::Push(5), Op::Bin(Ult), Op::Bin(Bor)]);
    }
    /// slot += 1
    fn inc(&mut self, s: u32) {
        self.o(&[Op::Load(s), Op::Push(1), Op::Bin(Add), Op::Store(s)]);
    }
}

/// Pushes the lowercase hex digit of bits sh..sh+4 of slot c.
fn hex(c: u32, sh: i64) -> Vec<Op> {
    let d = vec![Op::Load(c), Op::Push(sh), Op::Bin(Shr), Op::Push(15), Op::Bin(Band)];
    let mut v = d.clone();
    v.extend([Op::Push(48), Op::Bin(Add)]);
    v.extend(d);
    v.extend([Op::Push(9), Op::Bin(Gt), Op::Push(39), Op::Bin(Mul), Op::Bin(Add)]);
    v
}

/// The string library function of this name, if it is one.
pub fn make(name: &str) -> Option<Func> {
    let mut b = B::new();
    Some(match name {
        // (n): a new string of n bytes (zeros), its length set
        "lib.str_new" => {
            b.o(&[Op::Load(0), Op::Push(8), Op::Bin(Add), call("lib.bytes", 1), Op::Store(1)]);
            b.o(&[Op::Load(1), Op::Load(0), Op::StoreM(8), Op::Load(1), Op::Ret]);
            b.done(name, 1, 2)
        }
        // (s, lo, n): a new string of s's n bytes from lo (checked by the caller)
        "lib.str_sub" => {
            b.o(&[Op::Load(2), call("lib.str_new", 1), Op::Store(3)]);
            b.o(&[Op::Load(3), Op::Push(8), Op::Bin(Add), Op::Load(0), Op::Push(8), Op::Bin(Add), Op::Load(1), Op::Bin(Add), Op::Load(2), call("lib.copy", 3), Op::Pop]);
            b.o(&[Op::Load(3), Op::Ret]);
            b.done(name, 3, 4)
        }
        // (a, b): a's bytes then b's
        "lib.str_concat" => {
            b.field(0, 0);
            b.o(&[Op::Store(2), Op::Load(2)]);
            b.field(1, 0);
            b.o(&[Op::Bin(Add), call("lib.str_new", 1), Op::Store(3)]);
            b.o(&[Op::Load(3), Op::Push(8), Op::Bin(Add), Op::Load(0), Op::Push(8), Op::Bin(Add), Op::Load(2), call("lib.copy", 3), Op::Pop]);
            b.o(&[Op::Load(3), Op::Push(8), Op::Bin(Add), Op::Load(2), Op::Bin(Add), Op::Load(1), Op::Push(8), Op::Bin(Add)]);
            b.field(1, 0);
            b.o(&[call("lib.copy", 3), Op::Pop, Op::Load(3), Op::Ret]);
            b.done(name, 2, 4)
        }
        // (s, n): Python's s * n, empty for n <= 0
        "lib.str_repeat" => {
            let (pos, top, end) = (b.label(), b.label(), b.label());
            b.o(&[Op::Load(1), Op::Push(0), Op::Bin(Lt), Op::Jz(pos), Op::Push(0), Op::Store(1), Op::Label(pos)]);
            b.field(0, 0);
            b.o(&[Op::Store(2), Op::Load(2), Op::Load(1), Op::Bin(MulOv), Op::Store(5)]);
            b.o(&[Op::Load(5), call("lib.str_new", 1), Op::Store(3), Op::Load(5), Op::Jz(end)]);
            b.o(&[Op::Label(top), Op::Load(4), Op::Load(1), Op::Bin(Lt), Op::Jz(end)]);
            b.o(&[Op::Load(3), Op::Push(8), Op::Bin(Add), Op::Load(4), Op::Load(2), Op::Bin(Mul), Op::Bin(Add), Op::Load(0), Op::Push(8), Op::Bin(Add), Op::Load(2), call("lib.copy", 3), Op::Pop]);
            b.inc(4);
            b.o(&[Op::Jmp(top), Op::Label(end), Op::Load(3), Op::Ret]);
            b.done(name, 2, 6)
        }
        // (a, b): 1 if they hold the same bytes, else 0
        "lib.str_eq" => {
            let (same, top, next, yes) = (b.label(), b.label(), b.label(), b.label());
            b.field(0, 0);
            b.o(&[Op::Store(3), Op::Load(3)]);
            b.field(1, 0);
            b.o(&[Op::Bin(Eq), Op::Jnz(same), Op::Push(0), Op::Ret, Op::Label(same)]);
            b.o(&[Op::Label(top), Op::Load(2), Op::Load(3), Op::Bin(Lt), Op::Jz(yes)]);
            b.sbyte(0, 2);
            b.sbyte(1, 2);
            b.o(&[Op::Bin(Ne), Op::Jz(next), Op::Push(0), Op::Ret, Op::Label(next)]);
            b.inc(2);
            b.o(&[Op::Jmp(top), Op::Label(yes), Op::Push(1), Op::Ret]);
            b.done(name, 2, 4)
        }
        // (a, b): -1, 0 or 1 as a is before, the same as or after b, byte
        // by byte (Go's order, and Python's for ASCII)
        "lib.str_cmp" => {
            let (shorter, top, next, gt, tail) = (b.label(), b.label(), b.label(), b.label(), b.label());
            b.field(0, 0);
            b.o(&[Op::Store(3)]);
            b.field(1, 0);
            b.o(&[Op::Store(4), Op::Load(3), Op::Store(5), Op::Load(4), Op::Load(3), Op::Bin(Lt), Op::Jz(shorter), Op::Load(4), Op::Store(5), Op::Label(shorter)]);
            b.o(&[Op::Label(top), Op::Load(2), Op::Load(5), Op::Bin(Lt), Op::Jz(tail)]);
            b.sbyte(0, 2);
            b.o(&[Op::Store(6)]);
            b.sbyte(1, 2);
            b.o(&[Op::Store(7), Op::Load(6), Op::Load(7), Op::Bin(Eq), Op::Jnz(next)]);
            b.o(&[Op::Load(6), Op::Load(7), Op::Bin(Lt), Op::Jz(gt), Op::Push(-1), Op::Ret, Op::Label(gt), Op::Push(1), Op::Ret, Op::Label(next)]);
            b.inc(2);
            b.o(&[Op::Jmp(top), Op::Label(tail)]);
            b.o(&[Op::Load(3), Op::Load(4), Op::Bin(Gt), Op::Load(3), Op::Load(4), Op::Bin(Lt), Op::Bin(Sub), Op::Ret]);
            b.done(name, 2, 8)
        }
        // (s, lo, hi): Python's s[lo:hi], each side i64::MIN where it is
        // left out: negative from the end, then held to 0..len
        "lib.str_slice" => {
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
            b.o(&[Op::Load(0), Op::Load(1), Op::Load(2), Op::Load(1), Op::Bin(Sub), call("lib.str_sub", 3), Op::Ret]);
            b.done(name, 3, 4)
        }
        // (s): its bytes on standard output, where they are
        "lib.str_print" => {
            b.o(&[Op::Load(0), Op::Push(8), Op::Bin(Add)]);
            b.field(0, 0);
            b.o(&[call(PRINT, 2), Op::Pop, Op::Push(0), Op::Ret]);
            b.done(name, 1, 1)
        }
        // (n): its decimal digits (Python's str(n), Go's strconv.Itoa); the
        // digits are worked out from the number made negative, so the
        // lowest number has its own
        "lib.int_str" => {
            let (neg, go, count, digit, done) = (b.label(), b.label(), b.label(), b.label(), b.label());
            b.o(&[Op::Load(0), Op::Push(0), Op::Bin(Lt), Op::Store(1), Op::Load(1), Op::Jnz(neg)]);
            b.o(&[Op::Load(0), Op::Un(UnOp::Neg), Op::Store(2), Op::Jmp(go), Op::Label(neg), Op::Load(0), Op::Store(2), Op::Label(go)]);
            b.o(&[Op::Load(2), Op::Store(4), Op::Label(count)]);
            b.inc(3);
            b.o(&[Op::Load(4), Op::Push(10), Op::Bin(DivTrunc), Op::Store(4), Op::Load(4), Op::Jnz(count)]);
            b.o(&[Op::Load(3), Op::Load(1), Op::Bin(Add), call("lib.str_new", 1), Op::Store(5)]);
            b.o(&[Op::Load(3), Op::Load(1), Op::Bin(Add), Op::Push(1), Op::Bin(Sub), Op::Store(6), Op::Load(2), Op::Store(4), Op::Label(digit)]);
            b.sat(5, 6);
            b.o(&[Op::Push(48), Op::Load(4), Op::Push(10), Op::Bin(ModTrunc), Op::Bin(Sub), Op::StoreM(1)]);
            b.o(&[Op::Load(6), Op::Push(1), Op::Bin(Sub), Op::Store(6), Op::Load(4), Op::Push(10), Op::Bin(DivTrunc), Op::Store(4), Op::Load(4), Op::Jnz(digit)]);
            b.o(&[Op::Load(1), Op::Jz(done), Op::Load(5), Op::Push(8), Op::Bin(Add), Op::Push(45), Op::StoreM(1), Op::Label(done), Op::Load(5), Op::Ret]);
            b.done(name, 1, 7)
        }
        // (s): Python's int(s): blanks around it (as int() takes them: \t
        // to \r and space), a sign, digits with single underscores between
        // them; else ValueError. The number is built negative, so the
        // lowest one fits, and one past 64 bits stops as an overflow.
        "lib.str_int" => {
            let (s, i, e, neg, v, u, c) = (0, 1, 2, 3, 4, 5, 6);
            let (lead, lead_end, trail, trail_end, minus, plus, signed, top, under, digit, end, bad, pos) =
                (b.label(), b.label(), b.label(), b.label(), b.label(), b.label(), b.label(), b.label(), b.label(), b.label(), b.label(), b.label(), b.label());
            let int_blank = |b: &mut B, at: &[Op]| {
                b.o(at);
                b.o(&[Op::Store(c), Op::Load(c), Op::Push(9), Op::Bin(Sub), Op::Push(5), Op::Bin(Ult), Op::Load(c), Op::Push(32), Op::Bin(Eq), Op::Bin(Bor)]);
            };
            b.field(s, 0);
            b.o(&[Op::Store(e)]);
            b.o(&[Op::Label(lead), Op::Load(i), Op::Load(e), Op::Bin(Lt), Op::Jz(lead_end)]);
            int_blank(&mut b, &[Op::Load(s), Op::Push(8), Op::Bin(Add), Op::Load(i), Op::Bin(Add), Op::LoadM(1), Op::Push(255), Op::Bin(Band)]);
            b.o(&[Op::Jz(lead_end)]);
            b.inc(i);
            b.o(&[Op::Jmp(lead), Op::Label(lead_end)]);
            b.o(&[Op::Label(trail), Op::Load(e), Op::Load(i), Op::Bin(Gt), Op::Jz(trail_end)]);
            int_blank(&mut b, &[Op::Load(s), Op::Push(7), Op::Bin(Add), Op::Load(e), Op::Bin(Add), Op::LoadM(1), Op::Push(255), Op::Bin(Band)]);
            b.o(&[Op::Jz(trail_end), Op::Load(e), Op::Push(1), Op::Bin(Sub), Op::Store(e), Op::Jmp(trail), Op::Label(trail_end)]);
            b.o(&[Op::Load(i), Op::Load(e), Op::Bin(Lt), Op::Jz(bad)]);
            b.sbyte(s, i);
            b.o(&[Op::Store(c), Op::Load(c), Op::Push(45), Op::Bin(Eq), Op::Jz(minus), Op::Push(1), Op::Store(neg), Op::Jmp(plus), Op::Label(minus)]);
            b.o(&[Op::Load(c), Op::Push(43), Op::Bin(Eq), Op::Jz(signed), Op::Label(plus)]);
            b.inc(i);
            b.o(&[Op::Label(signed), Op::Push(1), Op::Store(u)]);
            b.o(&[Op::Label(top), Op::Load(i), Op::Load(e), Op::Bin(Lt), Op::Jz(end)]);
            b.sbyte(s, i);
            b.o(&[Op::Store(c), Op::Load(c), Op::Push(95), Op::Bin(Eq), Op::Jz(under)]);
            b.o(&[Op::Load(u), Op::Jnz(bad), Op::Push(1), Op::Store(u), Op::Jmp(digit), Op::Label(under)]);
            b.o(&[Op::Load(c), Op::Push(48), Op::Bin(Sub), Op::Push(10), Op::Bin(Ult), Op::Jz(bad)]);
            b.o(&[Op::Load(v), Op::Push(10), Op::Bin(MulOv), Op::Load(c), Op::Push(48), Op::Bin(Sub), Op::Bin(SubOv), Op::Store(v), Op::Push(0), Op::Store(u), Op::Label(digit)]);
            b.inc(i);
            b.o(&[Op::Jmp(top), Op::Label(end), Op::Load(u), Op::Jnz(bad)]);
            b.o(&[Op::Load(neg), Op::Jz(pos), Op::Load(v), Op::Ret, Op::Label(pos), Op::Load(v), Op::Un(UnOp::NegOv), Op::Ret]);
            b.o(&[Op::Label(bad), Op::Fail(1, NOT_AN_INT.to_vec())]);
            b.done(name, 1, 7)
        }
        // (c): the one-character string of ASCII c
        "lib.str_char" => {
            b.o(&[chars(), Op::Load(0), Op::Push(4), Op::Bin(Shl), Op::Bin(Add), Op::Ret]);
            b.done(name, 1, 1)
        }
        // (s, i): Python's s[i], negative from the end
        "lib.py_index" => {
            let (pos, ok) = (b.label(), b.label());
            b.o(&[Op::Load(1), Op::Push(0), Op::Bin(Lt), Op::Jz(pos), Op::Load(1)]);
            b.field(0, 0);
            b.o(&[Op::Bin(Add), Op::Store(1), Op::Label(pos), Op::Load(1)]);
            b.field(0, 0);
            b.o(&[Op::Bin(Ult), Op::Jnz(ok), Op::Fail(1, STR_INDEX.to_vec()), Op::Label(ok)]);
            b.sbyte(0, 1);
            b.o(&[call("lib.str_char", 1), Op::Ret]);
            b.done(name, 2, 2)
        }
        // (s): Python's ord(s) of a one-character string
        "lib.ord" => {
            let ok = b.label();
            b.field(0, 0);
            b.o(&[Op::Push(1), Op::Bin(Eq), Op::Jnz(ok), Op::Fail(1, ORD_ONE.to_vec()), Op::Label(ok), Op::Push(0), Op::Store(1)]);
            b.sbyte(0, 1);
            b.o(&[Op::Ret]);
            b.done(name, 1, 2)
        }
        // (n): Python's chr(n)
        "lib.chr" => {
            let (big, range) = (b.label(), b.label());
            b.o(&[Op::Load(0), Op::Push(128), Op::Bin(Ult), Op::Jz(big), Op::Load(0), call("lib.str_char", 1), Op::Ret, Op::Label(big)]);
            b.o(&[Op::Load(0), Op::Push(0x110000), Op::Bin(Ult), Op::Jz(range), Op::Fail(1, CHR_PAST_ASCII.to_vec()), Op::Label(range), Op::Fail(1, CHR_RANGE.to_vec())]);
            b.done(name, 1, 1)
        }
        // (): Python's input(): the bytes to the end of the line (\n, or
        // \r\n), or of the input; EOFError when there is nothing to read
        "lib.read_str" => {
            let (buf, cap, n, c, r, new) = (0, 1, 2, 3, 4, 5);
            let (top, not_end, not_cr, ascii, room, done) = (b.label(), b.label(), b.label(), b.label(), b.label(), b.label());
            b.o(&[Op::Push(16), Op::Store(cap), Op::Load(cap), call("lib.bytes", 1), Op::Store(buf)]);
            b.o(&[Op::Label(top), call(GETC, 0), Op::Store(c)]);
            b.o(&[Op::Load(c), Op::Push(-1), Op::Bin(Eq), Op::Jz(not_end), Op::Load(n), Op::Jnz(done), Op::Fail(1, EOF_ERROR.to_vec()), Op::Label(not_end)]);
            b.o(&[Op::Load(c), Op::Push(10), Op::Bin(Eq), Op::Jnz(done)]);
            b.o(&[Op::Load(c), Op::Push(13), Op::Bin(Eq), Op::Jz(not_cr), call(GETC, 0), Op::Store(c)]);
            b.o(&[Op::Load(c), Op::Push(10), Op::Bin(Eq), Op::Jnz(done), Op::Load(c), Op::Push(-1), Op::Bin(Eq), Op::Jnz(done), Op::Fail(1, LONE_CR.to_vec()), Op::Label(not_cr)]);
            b.o(&[Op::Load(c), Op::Push(128), Op::Bin(Ult), Op::Jnz(ascii), Op::Fail(1, NOT_ASCII_IN.to_vec()), Op::Label(ascii)]);
            b.o(&[Op::Load(n), Op::Load(cap), Op::Bin(Lt), Op::Jnz(room)]);
            b.o(&[Op::Load(cap), Op::Push(1), Op::Bin(Shl), Op::Store(cap), Op::Load(cap), call("lib.bytes", 1), Op::Store(new)]);
            b.o(&[Op::Load(new), Op::Load(buf), Op::Load(n), call("lib.copy", 3), Op::Pop, Op::Load(new), Op::Store(buf), Op::Label(room)]);
            b.o(&[Op::Load(buf), Op::Load(n), Op::Bin(Add), Op::Load(c), Op::StoreM(1)]);
            b.inc(n);
            b.o(&[Op::Jmp(top), Op::Label(done)]);
            b.o(&[Op::Load(n), call("lib.str_new", 1), Op::Store(r), Op::Load(r), Op::Push(8), Op::Bin(Add), Op::Load(buf), Op::Load(n), call("lib.copy", 3), Op::Pop, Op::Load(r), Op::Ret]);
            b.done(name, 0, 6)
        }
        // (s): Python's repr(s), as print shows a string in a list
        "lib.py_repr" => quote(name, true),
        // (s): Go's %q
        "lib.go_quote" => quote(name, false),
        // (s): s printed as Python prints it in a list
        "lib.py_repr_print" => {
            b.o(&[Op::Load(0), call("lib.py_repr", 1), call("lib.str_print", 1), Op::Ret]);
            b.done(name, 1, 1)
        }
        // (r): the UTF-8 bytes of the code point r (Go's string(r), %c);
        // one that is not a code point is U+FFFD, as Go makes it
        "lib.rune_str" => {
            let (bad, ok, two, three, four) = (b.label(), b.label(), b.label(), b.label(), b.label());
            b.o(&[Op::Load(0), Op::Push(0x110000), Op::Bin(Ult), Op::Jz(bad)]);
            b.o(&[Op::Load(0), Op::Push(0xD800), Op::Bin(Sub), Op::Push(0x800), Op::Bin(Ult), Op::Jz(ok), Op::Label(bad), Op::Push(0xFFFD), Op::Store(0), Op::Label(ok)]);
            let tails = |b: &mut B, n: u32| {
                for k in 1..n {
                    let sh = 6 * (n - 1 - k) as i64;
                    b.o(&[Op::Push(k as i64), Op::Store(2)]);
                    b.sat(1, 2);
                    b.o(&[Op::Load(0), Op::Push(sh), Op::Bin(Shr), Op::Push(63), Op::Bin(Band), Op::Push(0x80), Op::Bin(Bor), Op::StoreM(1)]);
                }
            };
            for (n, limit, next, lead) in [(1u32, 0x80i64, two, 0i64), (2, 0x800, three, 0xC0), (3, 0x10000, four, 0xE0)] {
                b.o(&[Op::Load(0), Op::Push(limit), Op::Bin(Lt), Op::Jz(next), Op::Push(n as i64), call("lib.str_new", 1), Op::Store(1)]);
                b.sat(1, 2);
                b.o(&[Op::Load(0), Op::Push(6 * (n as i64 - 1)), Op::Bin(Shr), Op::Push(lead), Op::Bin(Bor), Op::StoreM(1)]);
                tails(&mut b, n);
                b.o(&[Op::Load(1), Op::Ret, Op::Label(next)]);
            }
            b.o(&[Op::Push(4), call("lib.str_new", 1), Op::Store(1), Op::Push(0), Op::Store(2)]);
            b.sat(1, 2);
            b.o(&[Op::Load(0), Op::Push(18), Op::Bin(Shr), Op::Push(0xF0), Op::Bin(Bor), Op::StoreM(1)]);
            tails(&mut b, 4);
            b.o(&[Op::Load(1), Op::Ret]);
            b.done(name, 1, 3)
        }
        // (s, i): the code point at byte i as Go's range decodes it, and its
        // bytes: rune | size << 32 (U+FFFD and 1 where the bytes are not
        // UTF-8, as unicode/utf8.DecodeRuneInString says)
        "lib.go_decode" => {
            let (s, i, l, b0, x, k) = (0, 1, 2, 3, 4, 5);
            let bad = b.label();
            b.field(s, 0);
            b.o(&[Op::Store(l)]);
            b.sbyte(s, i);
            let ascii = b.label();
            b.o(&[Op::Store(b0), Op::Load(b0), Op::Push(0x80), Op::Bin(Lt), Op::Jz(ascii), Op::Load(b0), Op::Push(1 << 32), Op::Bin(Bor), Op::Ret, Op::Label(ascii)]);
            // the first byte, the second's range, the length
            let rows: [(i64, i64, i64, i64, u32); 8] = [
                (0xC2, 0xDF, 0x80, 0xBF, 2),
                (0xE0, 0xE0, 0xA0, 0xBF, 3),
                (0xE1, 0xEC, 0x80, 0xBF, 3),
                (0xED, 0xED, 0x80, 0x9F, 3),
                (0xEE, 0xEF, 0x80, 0xBF, 3),
                (0xF0, 0xF0, 0x90, 0xBF, 4),
                (0xF1, 0xF3, 0x80, 0xBF, 4),
                (0xF4, 0xF4, 0x80, 0x8F, 4),
            ];
            let marks = [0x1F, 0x0F, 0x07];
            for (lo0, hi0, lo1, hi1, n) in rows {
                let next = b.label();
                b.o(&[Op::Load(b0), Op::Push(lo0), Op::Bin(Sub), Op::Push(hi0 - lo0 + 1), Op::Bin(Ult), Op::Jz(next)]);
                b.o(&[Op::Load(b0), Op::Push(marks[n as usize - 2]), Op::Bin(Band), Op::Store(x)]);
                for j in 1..n {
                    let (lo, hi) = if j == 1 { (lo1, hi1) } else { (0x80, 0xBF) };
                    b.o(&[Op::Load(i), Op::Push(j as i64), Op::Bin(Add), Op::Store(k), Op::Load(k), Op::Load(l), Op::Bin(Lt), Op::Jz(bad)]);
                    b.sbyte(s, k);
                    b.o(&[Op::Store(k), Op::Load(k), Op::Push(lo), Op::Bin(Sub), Op::Push(hi - lo + 1), Op::Bin(Ult), Op::Jz(bad)]);
                    b.o(&[Op::Load(x), Op::Push(6), Op::Bin(Shl), Op::Load(k), Op::Push(63), Op::Bin(Band), Op::Bin(Bor), Op::Store(x)]);
                }
                b.o(&[Op::Load(x), Op::Push((n as i64) << 32), Op::Bin(Bor), Op::Ret, Op::Label(next)]);
            }
            b.o(&[Op::Label(bad), Op::Push(0xFFFD | 1 << 32), Op::Ret]);
            b.done(name, 2, 6)
        }
        // (s): Python's s.upper() and s.lower() (ASCII)
        "lib.str_upper" | "lib.str_lower" => {
            let (lo, sh) = if name == "lib.str_upper" { (97, -32) } else { (65, 32) };
            let (top, end, keep) = (b.label(), b.label(), b.label());
            b.field(0, 0);
            b.o(&[Op::Store(1), Op::Load(1), call("lib.str_new", 1), Op::Store(2)]);
            b.o(&[Op::Label(top), Op::Load(3), Op::Load(1), Op::Bin(Lt), Op::Jz(end)]);
            b.sbyte(0, 3);
            b.o(&[Op::Store(4), Op::Load(4), Op::Push(lo), Op::Bin(Sub), Op::Push(26), Op::Bin(Ult), Op::Jz(keep), Op::Load(4), Op::Push(sh), Op::Bin(Add), Op::Store(4), Op::Label(keep)]);
            b.sat(2, 3);
            b.o(&[Op::Load(4), Op::StoreM(1)]);
            b.inc(3);
            b.o(&[Op::Jmp(top), Op::Label(end), Op::Load(2), Op::Ret]);
            b.done(name, 1, 5)
        }
        // (s): Python's s.strip(), the blanks str.isspace() says
        "lib.str_strip" => {
            let (s, e, i, c) = (0, 1, 2, 3);
            let (lead, lead_end, trail, trail_end) = (b.label(), b.label(), b.label(), b.label());
            b.field(s, 0);
            b.o(&[Op::Store(e), Op::Label(lead), Op::Load(i), Op::Load(e), Op::Bin(Lt), Op::Jz(lead_end)]);
            b.sbyte(s, i);
            b.o(&[Op::Store(c)]);
            b.blank(c);
            b.o(&[Op::Jz(lead_end)]);
            b.inc(i);
            b.o(&[Op::Jmp(lead), Op::Label(lead_end)]);
            b.o(&[Op::Label(trail), Op::Load(e), Op::Load(i), Op::Bin(Gt), Op::Jz(trail_end)]);
            b.o(&[Op::Load(s), Op::Push(7), Op::Bin(Add), Op::Load(e), Op::Bin(Add), Op::LoadM(1), Op::Push(255), Op::Bin(Band), Op::Store(c)]);
            b.blank(c);
            b.o(&[Op::Jz(trail_end), Op::Load(e), Op::Push(1), Op::Bin(Sub), Op::Store(e), Op::Jmp(trail), Op::Label(trail_end)]);
            b.o(&[Op::Load(s), Op::Load(i), Op::Load(e), Op::Load(i), Op::Bin(Sub), call("lib.str_sub", 3), Op::Ret]);
            b.done(name, 1, 4)
        }
        // (s): Python's s.split(): the words between runs of blanks
        "lib.str_split" => {
            let (s, l, i, j, c, out) = (0, 1, 2, 3, 4, 5);
            let (top, skip, word, word_end, end) = (b.label(), b.label(), b.label(), b.label(), b.label());
            b.field(s, 0);
            b.o(&[Op::Store(l), Op::Push(0), call("lib.list_new", 1), Op::Store(out)]);
            b.o(&[Op::Label(top), Op::Label(skip), Op::Load(i), Op::Load(l), Op::Bin(Lt), Op::Jz(end)]);
            b.sbyte(s, i);
            b.o(&[Op::Store(c)]);
            b.blank(c);
            let found = b.label();
            b.o(&[Op::Jz(found)]);
            b.inc(i);
            b.o(&[Op::Jmp(skip), Op::Label(found), Op::Load(i), Op::Store(j)]);
            b.o(&[Op::Label(word), Op::Load(j), Op::Load(l), Op::Bin(Lt), Op::Jz(word_end)]);
            b.sbyte(s, j);
            b.o(&[Op::Store(c)]);
            b.blank(c);
            b.o(&[Op::Jnz(word_end)]);
            b.inc(j);
            b.o(&[Op::Jmp(word), Op::Label(word_end)]);
            b.o(&[Op::Load(out), Op::Load(s), Op::Load(i), Op::Load(j), Op::Load(i), Op::Bin(Sub), call("lib.str_sub", 3), call("lib.list_append", 2), Op::Pop]);
            b.o(&[Op::Load(j), Op::Store(i), Op::Jmp(top), Op::Label(end), Op::Load(out), Op::Ret]);
            b.done(name, 1, 6)
        }
        // (sep, list): Python's sep.join(list) of strings
        "lib.str_join" => {
            let (sep, list, n, k, total, r, p, e) = (0, 1, 2, 3, 4, 5, 6, 7);
            let (sum, sum_end, top, first, end) = (b.label(), b.label(), b.label(), b.label(), b.label());
            b.field(list, 0);
            b.o(&[Op::Store(n)]);
            b.o(&[Op::Label(sum), Op::Load(k), Op::Load(n), Op::Bin(Lt), Op::Jz(sum_end), Op::Load(total)]);
            b.field(list, 16);
            b.o(&[Op::Load(k), Op::Push(3), Op::Bin(Shl), Op::Bin(Add), Op::LoadM(8), Op::LoadM(8), Op::Bin(Add), Op::Store(total)]);
            let no_sep = b.label();
            b.o(&[Op::Load(k), Op::Jz(no_sep), Op::Load(total)]);
            b.field(sep, 0);
            b.o(&[Op::Bin(Add), Op::Store(total), Op::Label(no_sep)]);
            b.inc(k);
            b.o(&[Op::Jmp(sum), Op::Label(sum_end), Op::Load(total), call("lib.str_new", 1), Op::Store(r), Op::Push(0), Op::Store(k)]);
            b.o(&[Op::Label(top), Op::Load(k), Op::Load(n), Op::Bin(Lt), Op::Jz(end), Op::Load(k), Op::Jz(first)]);
            b.sat(r, p);
            b.o(&[Op::Load(sep), Op::Push(8), Op::Bin(Add)]);
            b.field(sep, 0);
            b.o(&[call("lib.copy", 3), Op::Pop, Op::Load(p)]);
            b.field(sep, 0);
            b.o(&[Op::Bin(Add), Op::Store(p), Op::Label(first)]);
            b.field(list, 16);
            b.o(&[Op::Load(k), Op::Push(3), Op::Bin(Shl), Op::Bin(Add), Op::LoadM(8), Op::Store(e)]);
            b.sat(r, p);
            b.o(&[Op::Load(e), Op::Push(8), Op::Bin(Add)]);
            b.field(e, 0);
            b.o(&[call("lib.copy", 3), Op::Pop, Op::Load(p)]);
            b.field(e, 0);
            b.o(&[Op::Bin(Add), Op::Store(p)]);
            b.inc(k);
            b.o(&[Op::Jmp(top), Op::Label(end), Op::Load(r), Op::Ret]);
            b.done(name, 2, 8)
        }
        // (s, sub): where sub first is in s, or -1 (Python's s.find(sub);
        // `in` is find >= 0)
        "lib.str_find" => {
            let (s, sub, l, m, i, j) = (0, 1, 2, 3, 4, 5);
            let (top, end, inner, found, miss) = (b.label(), b.label(), b.label(), b.label(), b.label());
            b.field(s, 0);
            b.o(&[Op::Store(l)]);
            b.field(sub, 0);
            b.o(&[Op::Store(m), Op::Label(top), Op::Load(i), Op::Load(m), Op::Bin(Add), Op::Load(l), Op::Bin(Le), Op::Jz(end), Op::Push(0), Op::Store(j)]);
            b.o(&[Op::Label(inner), Op::Load(j), Op::Load(m), Op::Bin(Lt), Op::Jz(found)]);
            b.o(&[Op::Load(s), Op::Push(8), Op::Bin(Add), Op::Load(i), Op::Bin(Add), Op::Load(j), Op::Bin(Add), Op::LoadM(1)]);
            b.o(&[Op::Load(sub), Op::Push(8), Op::Bin(Add), Op::Load(j), Op::Bin(Add), Op::LoadM(1), Op::Bin(Ne), Op::Jnz(miss)]);
            b.inc(j);
            b.o(&[Op::Jmp(inner), Op::Label(found), Op::Load(i), Op::Ret, Op::Label(miss)]);
            b.inc(i);
            b.o(&[Op::Jmp(top), Op::Label(end), Op::Push(-1), Op::Ret]);
            b.done(name, 2, 6)
        }
        // (s, p): 1 if s begins with p (Python's s.startswith(p)), else 0
        "lib.str_startswith" => {
            let (s, p, m, j) = (0, 1, 2, 3);
            let (fits, top, yes) = (b.label(), b.label(), b.label());
            b.field(p, 0);
            b.o(&[Op::Store(m), Op::Load(m)]);
            b.field(s, 0);
            b.o(&[Op::Bin(Le), Op::Jnz(fits), Op::Push(0), Op::Ret, Op::Label(fits)]);
            b.o(&[Op::Label(top), Op::Load(j), Op::Load(m), Op::Bin(Lt), Op::Jz(yes)]);
            b.sbyte(s, j);
            b.sbyte(p, j);
            let same = b.label();
            b.o(&[Op::Bin(Eq), Op::Jnz(same), Op::Push(0), Op::Ret, Op::Label(same)]);
            b.inc(j);
            b.o(&[Op::Jmp(top), Op::Label(yes), Op::Push(1), Op::Ret]);
            b.done(name, 2, 4)
        }
        // (p): C's strlen: the bytes before the first zero
        "lib.c_strlen" => {
            let (top, end) = (b.label(), b.label());
            b.o(&[Op::Label(top), Op::Load(0), Op::Load(1), Op::Bin(Add), Op::LoadM(1), Op::Jz(end)]);
            b.inc(1);
            b.o(&[Op::Jmp(top), Op::Label(end), Op::Load(1), Op::Ret]);
            b.done(name, 1, 2)
        }
        // (a, b): C's strcmp as Windows' C library gives it: -1, 0 or 1,
        // the bytes read unsigned
        "lib.c_strcmp" => {
            let (a, bb, i, x, y) = (0, 1, 2, 3, 4);
            let (top, same, gt) = (b.label(), b.label(), b.label());
            b.o(&[Op::Label(top), Op::Load(a), Op::Load(i), Op::Bin(Add), Op::LoadM(1), Op::Push(255), Op::Bin(Band), Op::Store(x)]);
            b.o(&[Op::Load(bb), Op::Load(i), Op::Bin(Add), Op::LoadM(1), Op::Push(255), Op::Bin(Band), Op::Store(y)]);
            b.o(&[Op::Load(x), Op::Load(y), Op::Bin(Eq), Op::Jnz(same), Op::Load(x), Op::Load(y), Op::Bin(Lt), Op::Jz(gt), Op::Push(-1), Op::Ret, Op::Label(gt), Op::Push(1), Op::Ret]);
            let more = b.label();
            b.o(&[Op::Label(same), Op::Load(x), Op::Jnz(more), Op::Push(0), Op::Ret, Op::Label(more)]);
            b.inc(i);
            b.o(&[Op::Jmp(top)]);
            b.done(name, 2, 5)
        }
        // (p): C's chars to their zero, printed (printf's %s, puts)
        "lib.c_print" => {
            b.o(&[Op::Load(0), Op::Load(0), call("lib.c_strlen", 1), call(PRINT, 2), Op::Pop, Op::Push(0), Op::Ret]);
            b.done(name, 1, 1)
        }
        // (c): the byte c printed (C's %c: the int made an unsigned char)
        "lib.print_byte" => {
            b.o(&[Op::Frame(0), Op::Load(0), Op::StoreM(1), Op::Frame(0), Op::Push(1), call(PRINT, 2), Op::Pop, Op::Push(0), Op::Ret]);
            let mut f = b.done(name, 1, 1);
            f.frame = 8;
            f
        }
        _ => return None,
    })
}

/// Python's repr of a string (quotes chosen as Python chooses them) or Go's
/// %q, as a new string: at most four bytes for each byte, and the quotes.
fn quote(name: &str, python: bool) -> Func {
    let mut b = B::new();
    let (s, q, i, l, buf, n, c, has1, has2) = (0, 1, 2, 3, 4, 5, 6, 7, 8);
    let (scan, scan_end, top, end) = (b.label(), b.label(), b.label(), b.label());
    b.field(s, 0);
    b.o(&[Op::Store(l), Op::Load(l), Op::Push(4), Op::Bin(MulOv), Op::Push(2), Op::Bin(AddOv), call("lib.str_new", 1), Op::Store(buf)]);
    if python {
        // ' unless the string holds ' and no "
        let (not1, not2, chosen) = (b.label(), b.label(), b.label());
        b.o(&[Op::Label(scan), Op::Load(i), Op::Load(l), Op::Bin(Lt), Op::Jz(scan_end)]);
        b.sbyte(s, i);
        b.o(&[Op::Store(c), Op::Load(c), Op::Push(39), Op::Bin(Eq), Op::Jz(not1), Op::Push(1), Op::Store(has1), Op::Label(not1)]);
        b.o(&[Op::Load(c), Op::Push(34), Op::Bin(Eq), Op::Jz(not2), Op::Push(1), Op::Store(has2), Op::Label(not2)]);
        b.inc(i);
        b.o(&[Op::Jmp(scan), Op::Label(scan_end), Op::Push(0), Op::Store(i), Op::Push(39), Op::Store(q)]);
        b.o(&[Op::Load(has1), Op::Jz(chosen), Op::Load(has2), Op::Jnz(chosen), Op::Push(34), Op::Store(q), Op::Label(chosen)]);
    } else {
        b.o(&[Op::Label(scan), Op::Label(scan_end), Op::Push(34), Op::Store(q)]);
    }
    b.put(buf, n, &[Op::Load(q)]);
    b.o(&[Op::Label(top), Op::Load(i), Op::Load(l), Op::Bin(Lt), Op::Jz(end)]);
    b.sbyte(s, i);
    b.o(&[Op::Store(c)]);
    let next = b.label();
    // the escapes the language writes with a letter
    let mut letters: Vec<(i64, u8)> = vec![(9, b't'), (10, b'n'), (13, b'r')];
    if !python {
        letters.extend([(7, b'a'), (8, b'b'), (12, b'f'), (11, b'v')]);
    }
    for (ch, letter) in letters {
        let other = b.label();
        b.o(&[Op::Load(c), Op::Push(ch), Op::Bin(Eq), Op::Jz(other)]);
        b.put(buf, n, &[Op::Push(92)]);
        b.put(buf, n, &[Op::Push(letter as i64)]);
        b.o(&[Op::Jmp(next), Op::Label(other)]);
    }
    // a backslash, or the quote: a backslash before it
    let (not_escaped, plain) = (b.label(), b.label());
    b.o(&[Op::Load(c), Op::Push(92), Op::Bin(Eq), Op::Load(c), Op::Load(q), Op::Bin(Eq), Op::Bin(Bor), Op::Jz(not_escaped)]);
    b.put(buf, n, &[Op::Push(92)]);
    b.put(buf, n, &[Op::Load(c)]);
    b.o(&[Op::Jmp(next), Op::Label(not_escaped)]);
    let ascii = b.label();
    b.o(&[Op::Load(c), Op::Push(128), Op::Bin(Lt), Op::Jnz(ascii), Op::Fail(1, QUOTE_PAST_ASCII.to_vec()), Op::Label(ascii)]);
    b.o(&[Op::Load(c), Op::Push(32), Op::Bin(Sub), Op::Push(95), Op::Bin(Ult), Op::Jnz(plain)]);
    b.put(buf, n, &[Op::Push(92)]);
    b.put(buf, n, &[Op::Push(120)]);
    b.put(buf, n, &hex(c, 4));
    b.put(buf, n, &hex(c, 0));
    b.o(&[Op::Jmp(next), Op::Label(plain)]);
    b.put(buf, n, &[Op::Load(c)]);
    b.o(&[Op::Label(next)]);
    b.inc(i);
    b.o(&[Op::Jmp(top), Op::Label(end)]);
    b.put(buf, n, &[Op::Load(q)]);
    b.o(&[Op::Load(buf), Op::Load(n), Op::StoreM(8), Op::Load(buf), Op::Ret]);
    b.done(name, 1, 9)
}
