//! Strike IR: what every language becomes. A function is a list of
//! operations on a stack of 64-bit integers; its locals are numbered slots,
//! the parameters first. Nothing in it says which language it came from, so
//! the same function written in C, Go or Python is the same IR and has the
//! same brand.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    /// division rounding toward zero (C, Go)
    DivTrunc,
    /// the remainder of DivTrunc: it has the sign of the dividend
    ModTrunc,
    /// division rounding down (Python's //)
    DivFloor,
    /// the remainder of DivFloor: it has the sign of the divisor
    ModFloor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Band,
    Bor,
    Bxor,
    Shl,
    /// arithmetic shift right
    Shr,
    /// `&&`: 1 or 0, the right side only read when needed
    AndBool,
    OrBool,
    /// Python's `and`: the left value if it is false, else the right value
    AndValue,
    OrValue,
    /// Add, Sub, Mul and Shl whose result must fit 64 bits: a language
    /// whose numbers do not wrap (Python) stops with an error where they
    /// would, never giving a wrong number
    AddOv,
    SubOv,
    MulOv,
    ShlOv,
    /// below and below or equal, the numbers read unsigned: `i < n` and
    /// `0 <= i` in one test (an index checked against a length)
    Ult,
    Ule,
    /// Python's `/`: true division, which the lowering makes a float's (it
    /// never reaches the IR)
    DivTrue,
    /// IEEE double arithmetic on the bit patterns (a float is carried as
    /// its 64 bits), rounded to nearest as the machine rounds
    FAdd,
    FSub,
    FMul,
    FDiv,
    /// the remainder of a division cut toward zero, exact (C's fmod)
    FRem,
    /// comparisons of floats: false for a NaN, but FNe true
    FEq,
    FNe,
    FLt,
    FLe,
    FGt,
    FGe,
}

impl BinOp {
    pub fn parse(s: &str) -> Option<BinOp> {
        use BinOp::*;
        Some(match s {
            "add" => Add,
            "sub" => Sub,
            "mul" => Mul,
            "div_trunc" => DivTrunc,
            "mod_trunc" => ModTrunc,
            "div_floor" => DivFloor,
            "mod_floor" => ModFloor,
            "eq" => Eq,
            "ne" => Ne,
            "lt" => Lt,
            "le" => Le,
            "gt" => Gt,
            "ge" => Ge,
            "band" => Band,
            "bor" => Bor,
            "bxor" => Bxor,
            "shl" => Shl,
            "shr" => Shr,
            "and_bool" => AndBool,
            "or_bool" => OrBool,
            "and_value" => AndValue,
            "or_value" => OrValue,
            "ult" => Ult,
            "ule" => Ule,
            "div_true" => DivTrue,
            _ => return None,
        })
    }

    pub fn is_comparison(self) -> bool {
        use BinOp::*;
        matches!(self, Eq | Ne | Lt | Le | Gt | Ge | Ult | Ule)
    }

    /// An operation on floats (its numbers are bit patterns of doubles).
    pub fn is_float(self) -> bool {
        use BinOp::*;
        matches!(self, FAdd | FSub | FMul | FDiv | FRem | FEq | FNe | FLt | FLe | FGt | FGe)
    }

    pub fn is_float_comparison(self) -> bool {
        use BinOp::*;
        matches!(self, FEq | FNe | FLt | FLe | FGt | FGe)
    }

    /// The float operation for an integer one (C's `+` on doubles).
    pub fn float(self) -> Option<BinOp> {
        use BinOp::*;
        Some(match self {
            Add => FAdd,
            Sub => FSub,
            Mul => FMul,
            DivTrunc | DivTrue => FDiv,
            Eq => FEq,
            Ne => FNe,
            Lt => FLt,
            Le => FLe,
            Gt => FGt,
            Ge => FGe,
            _ => return None,
        })
    }

    /// The float operation worked out as the machine works it out (SSE2,
    /// and the x87 for FRem), bit for bit: also which NaN comes out. None
    /// for an operation that is not a float's.
    pub fn float_eval(self, a: i64, b: i64) -> Option<i64> {
        use BinOp::*;
        let (x, y) = (f64::from_bits(a as u64), f64::from_bits(b as u64));
        let r = match self {
            FAdd => x + y,
            FSub => x - y,
            FMul => x * y,
            FDiv => x / y,
            FRem => x % y,
            FEq => return Some((x == y) as i64),
            FNe => return Some((x != y) as i64),
            FLt => return Some((x < y) as i64),
            FLe => return Some((x <= y) as i64),
            FGt => return Some((x > y) as i64),
            FGe => return Some((x >= y) as i64),
            _ => return None,
        };
        if !r.is_nan() {
            return Some(r.to_bits() as i64);
        }
        // a NaN that comes in goes out quieted: the first one for SSE2,
        // the one with the larger fraction for the x87; an invalid
        // operation makes the machine's own NaN
        let pick = match (x.is_nan(), y.is_nan()) {
            (true, true) if self == FRem && (b as u64 & FRACTION) > (a as u64 & FRACTION) => b,
            (true, _) => a,
            (false, true) => b,
            (false, false) => return Some(DEFAULT_NAN),
        };
        Some(pick | QUIET)
    }

    /// The operation that stops where the number would not fit, for a
    /// language whose numbers do not wrap.
    pub fn checked(self) -> BinOp {
        match self {
            BinOp::Add => BinOp::AddOv,
            BinOp::Sub => BinOp::SubOv,
            BinOp::Mul => BinOp::MulOv,
            BinOp::Shl => BinOp::ShlOv,
            b => b,
        }
    }

    /// The result of a checked operation, or None where it does not fit.
    pub fn checked_eval(self, a: i64, b: i64) -> Option<Option<i64>> {
        Some(match self {
            BinOp::AddOv => a.checked_add(b),
            BinOp::SubOv => a.checked_sub(b),
            BinOp::MulOv => a.checked_mul(b),
            BinOp::ShlOv => {
                if (0..64).contains(&b) && (a << b) >> b == a {
                    Some(a << b)
                } else {
                    None
                }
            }
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum UnOp {
    Neg,
    Plus,
    /// 1 if zero, else 0
    Not,
    Bnot,
    /// Neg that stops on the lowest number (its negation does not fit)
    NegOv,
    /// a float's sign turned, and taken away (exact: only the sign bit)
    FNeg,
    FAbs,
    /// the square root, rounded as every operation is (exact for IEEE)
    FSqrt,
    /// a whole number made the nearest double, or the nearest float32
    /// (held as a double: one rounding, not two)
    FromInt,
    FromInt32,
    /// a double rounded to the nearest float32 (and held as a double again)
    ToF32,
    /// a float cut toward zero to a whole number, as the machine's cvttsd2si
    /// does: one that does not fit (or NaN) is the lowest number of 64 bits,
    /// or of 32 bits for ToInt32
    ToInt,
    ToInt32,
}

impl UnOp {
    pub fn parse(s: &str) -> Option<UnOp> {
        Some(match s {
            "neg" => UnOp::Neg,
            "plus" => UnOp::Plus,
            "not" => UnOp::Not,
            "bnot" => UnOp::Bnot,
            _ => return None,
        })
    }

    pub fn is_float(self) -> bool {
        use UnOp::*;
        matches!(self, FNeg | FAbs | FSqrt | FromInt | FromInt32 | ToF32 | ToInt | ToInt32)
    }

    /// The float operation as the machine works it out, bit for bit; None
    /// for one that is not a float's.
    pub fn float_eval(self, a: i64) -> Option<i64> {
        use UnOp::*;
        let x = f64::from_bits(a as u64);
        Some(match self {
            FNeg => a ^ i64::MIN,
            FAbs => a & i64::MAX,
            FSqrt if x.is_nan() => a | QUIET,
            FSqrt if x < 0.0 => DEFAULT_NAN,
            FSqrt => x.sqrt().to_bits() as i64,
            FromInt => (a as f64).to_bits() as i64,
            FromInt32 => ((a as f32) as f64).to_bits() as i64,
            // a NaN keeps its sign and the top of its fraction, quieted
            ToF32 if x.is_nan() => (a & (i64::MIN | 0x7FF0_0000_0000_0000 | (0x000F_FFFF_E000_0000))) | QUIET,
            ToF32 => ((x as f32) as f64).to_bits() as i64,
            ToInt if x.is_nan() || !(-9223372036854775808.0..9223372036854775808.0).contains(&x) => i64::MIN,
            ToInt => x as i64,
            ToInt32 if x.is_nan() || !(-2147483648.0..2147483648.0).contains(&x) => i32::MIN as i64,
            ToInt32 => x as i64,
            _ => return None,
        })
    }
}

/// The fraction bits of a double, its quiet bit, and the NaN the machine
/// makes for an invalid operation (negative, only the quiet bit set).
const FRACTION: u64 = (1 << 52) - 1;
const QUIET: i64 = 1 << 51;
pub const DEFAULT_NAN: i64 = 0xFFF8_0000_0000_0000_u64 as i64;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Op {
    Push(i64),
    Load(u32),
    /// pops into the slot
    Store(u32),
    /// pops the right, then the left, pushes the result (never a logic op:
    /// those become jumps)
    Bin(BinOp),
    Un(UnOp),
    /// pops the arguments (the last on top), pushes the result
    Call(String, u32),
    Jmp(u32),
    /// pops; jumps if it was zero
    Jz(u32),
    /// pops; jumps if it was not zero
    Jnz(u32),
    Label(u32),
    /// pops the value to return
    Ret,
    Pop,
    Dup,
    /// pops and prints in decimal
    PrintInt,
    PrintStr(Vec<u8>),
    /// the top cut to its low n bits and sign-extended back (a C int, a Go
    /// int32: what a result of that width is)
    Sext(u8),
    /// a program-wide variable (a C or Go global, a Python module name):
    /// pushed, or popped into
    LoadG(u32),
    StoreG(u32),
    /// reads a whole number from standard input and pushes it, then pushes
    /// 1; or, when there is none, pushes 0 and 0 (C's scanf, Go's Scan)
    ReadToken,
    /// reads a line from standard input and pushes the whole number it is;
    /// stops the program when it is not one (Python's int(input()))
    ReadLine,
    /// pops an address and pushes the n bytes there (1, 2, 4 or 8),
    /// sign-extended: an element of its own width (a C char, a Go int32)
    LoadM(u8),
    /// pops a value, then an address, and writes the value's low n bytes
    /// there
    StoreM(u8),
    /// pushes the address of byte n of the function's own memory (its C
    /// arrays: they live as long as the call)
    Frame(u32),
    /// pushes the address of global g (a C global array starts there)
    AddrG(u32),
    /// the message on standard error, and the program stops with the exit
    /// code (an index out of range: Python's IndexError, Go's panic)
    Fail(u8, Vec<u8>),
    /// pops a length, then an address, and prints that many bytes from
    /// there (a float's digits, made by the library)
    PrintMem,
    /// pops a length, then an address: those bytes on standard error, and
    /// the program stops with the exit code (a message the library made)
    FailMem(u8),
}

/// The runtime's allocator, called as a function: it pops a size in bytes
/// and pushes the address of that many new zero bytes (8-aligned). The
/// memory is never given back in this stage.
pub const ALLOC: &str = "rt.alloc";

#[derive(Clone, Debug)]
pub struct Func {
    /// the symbol (a language's own name, made unique)
    pub name: String,
    pub params: u32,
    pub slots: u32,
    pub code: Vec<Op>,
    /// bytes of the function's own memory (Op::Frame)
    pub frame: u32,
}

impl Func {
    pub fn new(name: impl Into<String>, params: u32, slots: u32, code: Vec<Op>) -> Func {
        Func { name: name.into(), params, slots, code, frame: 0 }
    }
}

/// v cut to its low n bits and sign-extended back.
pub fn sext(v: i64, n: u8) -> i64 {
    let s = 64 - n as u32;
    v.wrapping_shl(s).wrapping_shr(s)
}

/// How many loops hold each operation (a loop: the operations between a
/// label and a jump back to it).
pub fn loop_depths(code: &[Op]) -> Vec<usize> {
    let mut at = vec![0usize; code.len()];
    let mut label_at = std::collections::HashMap::new();
    for (i, op) in code.iter().enumerate() {
        if let Op::Label(l) = op {
            label_at.insert(*l, i);
        }
    }
    for (j, op) in code.iter().enumerate() {
        if let Op::Jmp(l) | Op::Jz(l) | Op::Jnz(l) = op {
            if let Some(&i) = label_at.get(l) {
                if i < j {
                    at[i..=j].iter_mut().for_each(|d| *d += 1);
                }
            }
        }
    }
    at
}

/// One more than the highest label of the code.
pub fn label_count(code: &[Op]) -> u32 {
    code.iter().filter_map(|o| if let Op::Label(l) = o { Some(*l + 1) } else { None }).max().unwrap_or(0)
}

/// The version of the code generator; a new one gives every brand anew.
pub const CODEGEN: &str = "strike-x64-4";

/// How the IR's stack changes at an operation (None: nothing comes after
/// it on the same way, a return or a stop).
pub fn stack_effect(op: &Op) -> Option<i32> {
    Some(match op {
        Op::Push(_) | Op::Load(_) | Op::Dup | Op::LoadG(_) | Op::ReadLine | Op::Frame(_) | Op::AddrG(_) => 1,
        Op::Store(_) | Op::Pop | Op::PrintInt | Op::Bin(_) | Op::Jz(_) | Op::Jnz(_) | Op::StoreG(_) => -1,
        Op::Un(_) | Op::Sext(_) | Op::PrintStr(_) | Op::Label(_) | Op::Jmp(_) | Op::LoadM(_) => 0,
        Op::StoreM(_) | Op::PrintMem => -2,
        Op::ReadToken => 2,
        Op::Call(_, n) => 1 - *n as i32,
        Op::Ret | Op::Fail(..) | Op::FailMem(_) => return None,
    })
}

impl Func {
    /// The brand of the function's machine code: the hash of everything that
    /// decides it (not its name: two equal functions share their code).
    pub fn brand(&self) -> String {
        let mut h = blake3::Hasher::new();
        h.update(CODEGEN.as_bytes());
        h.update(&self.params.to_le_bytes());
        h.update(&self.slots.to_le_bytes());
        h.update(&self.frame.to_le_bytes());
        for op in &self.code {
            let (tag, a): (u8, i64) = match op {
                Op::Push(v) => (1, *v),
                Op::Load(s) => (2, *s as i64),
                Op::Store(s) => (3, *s as i64),
                Op::Bin(b) => (4, *b as i64),
                Op::Un(u) => (5, *u as i64),
                Op::Call(name, n) => {
                    h.update(&[6]);
                    h.update(&(name.len() as u32).to_le_bytes());
                    h.update(name.as_bytes());
                    (6, *n as i64)
                }
                Op::Jmp(l) => (7, *l as i64),
                Op::Jz(l) => (8, *l as i64),
                Op::Jnz(l) => (9, *l as i64),
                Op::Label(l) => (10, *l as i64),
                Op::Ret => (11, 0),
                Op::Pop => (12, 0),
                Op::Dup => (13, 0),
                Op::PrintInt => (14, 0),
                Op::PrintStr(s) => {
                    h.update(&[15]);
                    h.update(&(s.len() as u32).to_le_bytes());
                    h.update(s);
                    (15, 0)
                }
                Op::Sext(n) => (16, *n as i64),
                Op::LoadG(g) => (17, *g as i64),
                Op::StoreG(g) => (18, *g as i64),
                Op::ReadToken => (19, 0),
                Op::ReadLine => (20, 0),
                Op::LoadM(w) => (21, *w as i64),
                Op::StoreM(w) => (22, *w as i64),
                Op::Frame(n) => (23, *n as i64),
                Op::AddrG(g) => (24, *g as i64),
                Op::Fail(code, m) => {
                    h.update(&[25]);
                    h.update(&(m.len() as u32).to_le_bytes());
                    h.update(m);
                    (25, *code as i64)
                }
                Op::PrintMem => (26, 0),
                Op::FailMem(code) => (27, *code as i64),
            };
            h.update(&[tag]);
            h.update(&a.to_le_bytes());
        }
        h.finalize().to_hex().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(x: f64) -> i64 {
        x.to_bits() as i64
    }

    /// What SSE2 gives, bit for bit: the first NaN that comes in, quieted,
    /// or the machine's own for an invalid operation; cvttsd2si's lowest
    /// number where a float does not fit.
    #[test]
    fn floats_are_worked_out_as_the_machine_works_them_out() {
        let snan = 0x7FF0_0000_0000_0001_i64;
        assert_eq!(BinOp::FAdd.float_eval(snan, f(1.0)), Some(snan | QUIET));
        assert_eq!(BinOp::FMul.float_eval(f(0.0), f(f64::INFINITY)), Some(DEFAULT_NAN));
        assert_eq!(BinOp::FSub.float_eval(f(f64::INFINITY), f(f64::INFINITY)), Some(DEFAULT_NAN));
        assert_eq!(UnOp::FSqrt.float_eval(f(-1.0)), Some(DEFAULT_NAN));
        assert_eq!(UnOp::FSqrt.float_eval(f(-0.0)), Some(f(-0.0)));
        assert_eq!(UnOp::ToInt.float_eval(f(1e19)), Some(i64::MIN));
        assert_eq!(UnOp::ToInt.float_eval(f(-2.9)), Some(-2));
        assert_eq!(UnOp::ToInt32.float_eval(f(3e9)), Some(i32::MIN as i64));
        assert_eq!(UnOp::ToF32.float_eval(f(0.1)), Some(f(0.1f32 as f64)));
        assert_eq!(UnOp::FromInt32.float_eval(16777217), Some(f(16777216.0)));
        assert_eq!(UnOp::FNeg.float_eval(f(0.0)), Some(f(-0.0)));
        assert_eq!(BinOp::FLt.float_eval(DEFAULT_NAN, f(1.0)), Some(0));
        assert_eq!(BinOp::FNe.float_eval(DEFAULT_NAN, DEFAULT_NAN), Some(1));
        assert_eq!(BinOp::FRem.float_eval(f(-7.5), f(2.0)), Some(f(-1.5)));
    }
}
