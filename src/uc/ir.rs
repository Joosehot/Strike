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
            _ => return None,
        })
    }

    pub fn is_comparison(self) -> bool {
        use BinOp::*;
        matches!(self, Eq | Ne | Lt | Le | Gt | Ge)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum UnOp {
    Neg,
    Plus,
    /// 1 if zero, else 0
    Not,
    Bnot,
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
}

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
}

#[derive(Clone, Debug)]
pub struct Func {
    /// the symbol (a language's own name, made unique)
    pub name: String,
    pub params: u32,
    pub slots: u32,
    pub code: Vec<Op>,
}

/// The version of the code generator; a new one gives every brand anew.
pub const CODEGEN: &str = "strike-x64-1";

impl Func {
    /// The brand of the function's machine code: the hash of everything that
    /// decides it (not its name: two equal functions share their code).
    pub fn brand(&self) -> String {
        let mut h = blake3::Hasher::new();
        h.update(CODEGEN.as_bytes());
        h.update(&self.params.to_le_bytes());
        h.update(&self.slots.to_le_bytes());
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
            };
            h.update(&[tag]);
            h.update(&a.to_le_bytes());
        }
        h.finalize().to_hex().to_string()
    }
}
