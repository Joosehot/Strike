//! Strike IR to x86-64 machine code, written byte by byte (no assembler, no
//! LLVM). One pass over a function: the IR's stack is the machine's stack,
//! and every slot has its place below rbp. Calls follow the Windows x64
//! convention (rcx, rdx, r8, r9, 32 bytes of shadow space, rsp aligned to 16
//! at the call), so the depth of the stack is known at every operation.

use super::ir::{BinOp, Func, Op, UnOp};

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// a function of the program, by symbol
    Func(String),
    /// a function of KERNEL32, through the import table
    Import(String),
    /// bytes in the read-only data
    Data(Vec<u8>),
}

/// A function's machine code, and the 32-bit fields still to be filled with
/// the distance to their target (each measured from the field's end).
#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub bytes: Vec<u8>,
    pub relocs: Vec<(u32, Target)>,
}

pub const IMPORTS: [&str; 3] = ["GetStdHandle", "WriteFile", "ExitProcess"];

pub(super) struct Asm {
    pub(super) b: Vec<u8>,
    labels: Vec<Option<u32>>,
    fixups: Vec<(u32, u32)>,
    relocs: Vec<(u32, Target)>,
}

impl Asm {
    pub(super) fn new() -> Asm {
        Asm { b: Vec::new(), labels: Vec::new(), fixups: Vec::new(), relocs: Vec::new() }
    }
    pub(super) fn e(&mut self, bytes: &[u8]) {
        self.b.extend_from_slice(bytes);
    }
    pub(super) fn d32(&mut self, v: i32) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    pub(super) fn label(&mut self) -> u32 {
        self.labels.push(None);
        self.labels.len() as u32 - 1
    }
    pub(super) fn bind(&mut self, l: u32) {
        self.labels[l as usize] = Some(self.b.len() as u32);
    }
    fn to(&mut self, l: u32) {
        self.fixups.push((self.b.len() as u32, l));
        self.d32(0);
    }
    pub(super) fn jmp(&mut self, l: u32) {
        self.e(&[0xE9]);
        self.to(l);
    }
    /// jcc rel32; cc is the condition's low nibble (4 = z, 5 = nz, 9 = ns)
    pub(super) fn jcc(&mut self, cc: u8, l: u32) {
        self.e(&[0x0F, 0x80 | cc]);
        self.to(l);
    }
    pub(super) fn reloc(&mut self, t: Target) {
        self.relocs.push((self.b.len() as u32, t));
        self.d32(0);
    }
    pub(super) fn call(&mut self, symbol: &str) {
        self.e(&[0xE8]);
        self.reloc(Target::Func(symbol.to_string()));
    }
    pub(super) fn call_import(&mut self, name: &str) {
        self.e(&[0xFF, 0x15]);
        self.reloc(Target::Import(name.to_string()));
    }
    pub(super) fn finish(mut self) -> Code {
        for (at, l) in std::mem::take(&mut self.fixups) {
            let target = self.labels[l as usize].expect("every label is placed") as i64;
            let rel = (target - (at as i64 + 4)) as i32;
            self.b[at as usize..at as usize + 4].copy_from_slice(&rel.to_le_bytes());
        }
        Code { bytes: self.b, relocs: self.relocs }
    }
    pub(super) fn sub_rsp(&mut self, n: i32) {
        self.e(&[0x48, 0x81, 0xEC]);
        self.d32(n);
    }
    pub(super) fn add_rsp(&mut self, n: i32) {
        self.e(&[0x48, 0x81, 0xC4]);
        self.d32(n);
    }
    pub(super) fn prologue(&mut self, frame: i32) {
        self.e(&[0x55, 0x48, 0x89, 0xE5]); // push rbp; mov rbp, rsp
        self.grow_stack(frame);
    }
    /// sub rsp, frame. Windows grows the stack one guard page at a time: a
    /// frame larger than a page touches every page on its way down.
    pub(super) fn grow_stack(&mut self, frame: i32) {
        let pages = frame / 4096;
        if pages > 0 {
            let top = self.label();
            self.e(&[0x41, 0xBB]); // mov r11d, pages
            self.d32(pages);
            self.bind(top);
            self.sub_rsp(4096);
            self.e(&[0x48, 0x85, 0x24, 0x24]); // test [rsp], rsp
            self.e(&[0x49, 0xFF, 0xCB]); // dec r11
            self.jcc(5, top);
        }
        if frame % 4096 > 0 {
            self.sub_rsp(frame % 4096);
        }
    }
    pub(super) fn epilogue(&mut self) {
        self.e(&[0x48, 0x89, 0xEC, 0x5D, 0xC3]); // mov rsp, rbp; pop rbp; ret
    }
}

fn slot(s: u32) -> i32 {
    -8 * (s as i32 + 1)
}

/// The machine code of one function.
pub fn compile(f: &Func) -> Code {
    let mut a = Asm::new();
    let labels = f.code.iter().filter_map(|o| if let Op::Label(l) = o { Some(*l + 1) } else { None }).max().unwrap_or(0);
    let labels_of: Vec<u32> = (0..labels).map(|_| a.label()).collect();
    let mut depth_at: Vec<Option<u32>> = vec![None; labels as usize];
    let frame = ((f.slots as i32 * 8) + 15) / 16 * 16;
    a.prologue(frame);
    // the parameters into their slots; every other slot starts at zero
    const STORE: [[u8; 3]; 4] = [[0x48, 0x89, 0x8D], [0x48, 0x89, 0x95], [0x4C, 0x89, 0x85], [0x4C, 0x89, 0x8D]];
    for i in 0..f.params {
        if i < 4 {
            a.e(&STORE[i as usize]);
        } else {
            a.e(&[0x48, 0x8B, 0x85]); // mov rax, [rbp + 48 + 8(i - 4)]
            a.d32(48 + 8 * (i as i32 - 4));
            a.e(&[0x48, 0x89, 0x85]);
        }
        a.d32(slot(i));
    }
    for s in f.params..f.slots {
        a.e(&[0x48, 0xC7, 0x85]); // mov qword [rbp + d], 0
        a.d32(slot(s));
        a.d32(0);
    }

    let mut depth: u32 = 0;
    for op in &f.code {
        match op {
            Op::Push(v) => {
                if let Ok(v) = i32::try_from(*v) {
                    a.e(&[0x68]);
                    a.d32(v);
                } else {
                    a.e(&[0x48, 0xB8]);
                    a.e(&v.to_le_bytes());
                    a.e(&[0x50]);
                }
                depth += 1;
            }
            Op::Load(s) => {
                a.e(&[0xFF, 0xB5]); // push qword [rbp + d]
                a.d32(slot(*s));
                depth += 1;
            }
            Op::Store(s) => {
                a.e(&[0x58, 0x48, 0x89, 0x85]); // pop rax; mov [rbp + d], rax
                a.d32(slot(*s));
                depth -= 1;
            }
            Op::Dup => {
                a.e(&[0xFF, 0x34, 0x24]);
                depth += 1;
            }
            Op::Pop => {
                a.e(&[0x48, 0x83, 0xC4, 0x08]);
                depth -= 1;
            }
            Op::Un(u) => {
                a.e(&[0x58]);
                match u {
                    UnOp::Neg => a.e(&[0x48, 0xF7, 0xD8]),
                    UnOp::Bnot => a.e(&[0x48, 0xF7, 0xD0]),
                    UnOp::Not => a.e(&[0x48, 0x85, 0xC0, 0x0F, 0x94, 0xC0, 0x0F, 0xB6, 0xC0]),
                    UnOp::Plus => {}
                }
                a.e(&[0x50]);
            }
            Op::Bin(b) => {
                a.e(&[0x59, 0x58]); // pop rcx (right); pop rax (left)
                let set = |a: &mut Asm, cc: u8| a.e(&[0x48, 0x39, 0xC8, 0x0F, cc, 0xC0, 0x0F, 0xB6, 0xC0]);
                match b {
                    BinOp::Add => a.e(&[0x48, 0x01, 0xC8]),
                    BinOp::Sub => a.e(&[0x48, 0x29, 0xC8]),
                    BinOp::Mul => a.e(&[0x48, 0x0F, 0xAF, 0xC1]),
                    BinOp::Band => a.e(&[0x48, 0x21, 0xC8]),
                    BinOp::Bor => a.e(&[0x48, 0x09, 0xC8]),
                    BinOp::Bxor => a.e(&[0x48, 0x31, 0xC8]),
                    BinOp::Shl => a.e(&[0x48, 0xD3, 0xE0]),
                    BinOp::Shr => a.e(&[0x48, 0xD3, 0xF8]),
                    BinOp::DivTrunc => a.e(&[0x48, 0x99, 0x48, 0xF7, 0xF9]),
                    BinOp::ModTrunc => a.e(&[0x48, 0x99, 0x48, 0xF7, 0xF9, 0x48, 0x89, 0xD0]),
                    BinOp::DivFloor | BinOp::ModFloor => {
                        let done = a.label();
                        a.e(&[0x48, 0x99, 0x48, 0xF7, 0xF9]); // cqo; idiv rcx
                        a.e(&[0x48, 0x85, 0xD2]); // test rdx, rdx
                        a.jcc(4, done);
                        a.e(&[0x49, 0x89, 0xD0, 0x49, 0x31, 0xC8]); // mov r8, rdx; xor r8, rcx
                        a.jcc(9, done); // the same signs: nothing to mend
                        a.e(&[0x48, 0xFF, 0xC8, 0x48, 0x01, 0xCA]); // dec rax; add rdx, rcx
                        a.bind(done);
                        if *b == BinOp::ModFloor {
                            a.e(&[0x48, 0x89, 0xD0]);
                        }
                    }
                    BinOp::Eq => set(&mut a, 0x94),
                    BinOp::Ne => set(&mut a, 0x95),
                    BinOp::Lt => set(&mut a, 0x9C),
                    BinOp::Le => set(&mut a, 0x9E),
                    BinOp::Gt => set(&mut a, 0x9F),
                    BinOp::Ge => set(&mut a, 0x9D),
                    BinOp::AndBool | BinOp::OrBool | BinOp::AndValue | BinOp::OrValue => {
                        unreachable!("logic operators are lowered to jumps")
                    }
                }
                a.e(&[0x50]);
                depth -= 1;
            }
            Op::Call(sym, n) => {
                call(&mut a, sym, *n, &mut depth, true);
            }
            Op::PrintInt => {
                call(&mut a, "rt.print_int", 1, &mut depth, false);
            }
            Op::PrintStr(s) => {
                a.e(&[0x48, 0x8D, 0x0D]); // lea rcx, [rip + data]
                a.reloc(Target::Data(s.clone()));
                a.e(&[0x48, 0xC7, 0xC2]); // mov rdx, len
                a.d32(s.len() as i32);
                call(&mut a, "rt.print_str", 0, &mut depth, false);
            }
            Op::Jmp(l) => {
                depth_at[*l as usize].get_or_insert(depth);
                a.jmp(labels_of[*l as usize]);
            }
            Op::Jz(l) | Op::Jnz(l) => {
                a.e(&[0x58, 0x48, 0x85, 0xC0]); // pop rax; test rax, rax
                depth -= 1;
                depth_at[*l as usize].get_or_insert(depth);
                a.jcc(if matches!(op, Op::Jz(_)) { 4 } else { 5 }, labels_of[*l as usize]);
            }
            Op::Label(l) => {
                // after a jump the depth is the one the jumps here carry
                if let Some(d) = depth_at[*l as usize] {
                    depth = d;
                } else {
                    depth_at[*l as usize] = Some(depth);
                }
                a.bind(labels_of[*l as usize]);
            }
            Op::Ret => {
                a.e(&[0x58]);
                a.epilogue();
                depth = depth.saturating_sub(1);
            }
        }
    }
    a.finish()
}

/// A call with `n` arguments on top of the stack (the first deepest); the
/// stack is aligned for it and the arguments taken off after it.
pub(super) fn call(a: &mut Asm, sym: &str, n: u32, depth: &mut u32, push_result: bool) {
    let k = n.saturating_sub(4);
    let mut s = 32 + 8 * k;
    if (*depth + s / 8) % 2 == 1 {
        s += 8;
    }
    a.sub_rsp(s as i32);
    let src = |i: u32| (s + 8 * (n - 1 - i)) as i32;
    for i in 4..n {
        a.e(&[0x48, 0x8B, 0x84, 0x24]); // mov rax, [rsp + src]
        a.d32(src(i));
        a.e(&[0x48, 0x89, 0x84, 0x24]); // mov [rsp + 32 + 8(i - 4)], rax
        a.d32(32 + 8 * (i as i32 - 4));
    }
    const LOAD: [[u8; 4]; 4] = [[0x48, 0x8B, 0x8C, 0x24], [0x48, 0x8B, 0x94, 0x24], [0x4C, 0x8B, 0x84, 0x24], [0x4C, 0x8B, 0x8C, 0x24]];
    for i in 0..n.min(4) {
        a.e(&LOAD[i as usize]);
        a.d32(src(i));
    }
    a.call(sym);
    a.add_rsp((s + 8 * n) as i32);
    *depth -= n;
    if push_result {
        a.e(&[0x50]);
        *depth += 1;
    }
}

/// The runtime every program gets: printing and the entry that calls the
/// program's entry and exits with what it returns.
pub fn runtime(entry: &str) -> Vec<(String, Code)> {
    // rt.print_str(rcx = bytes, rdx = length): WriteFile to standard output
    let mut a = Asm::new();
    a.prologue(64);
    a.e(&[0x48, 0x89, 0x4D, 0xF8]); // mov [rbp-8], rcx
    a.e(&[0x48, 0x89, 0x55, 0xF0]); // mov [rbp-16], rdx
    a.e(&[0x48, 0xC7, 0xC1, 0xF5, 0xFF, 0xFF, 0xFF]); // mov rcx, -11 (standard output)
    a.call_import("GetStdHandle");
    a.e(&[0x48, 0x89, 0xC1]); // mov rcx, rax
    a.e(&[0x48, 0x8B, 0x55, 0xF8]); // mov rdx, [rbp-8]
    a.e(&[0x4C, 0x8B, 0x45, 0xF0]); // mov r8, [rbp-16]
    a.e(&[0x4C, 0x8D, 0x4D, 0xE8]); // lea r9, [rbp-24]
    a.e(&[0x48, 0xC7, 0x44, 0x24, 0x20, 0, 0, 0, 0]); // mov qword [rsp+32], 0
    a.call_import("WriteFile");
    a.epilogue();
    let print_str = a.finish();

    // rt.print_int(rcx): decimal digits written backwards below rbp
    let mut a = Asm::new();
    let (digits, sign_done) = (a.label(), a.label());
    let positive = a.label();
    a.prologue(64);
    a.e(&[0x48, 0x89, 0xC8]); // mov rax, rcx
    a.e(&[0x45, 0x31, 0xC9]); // xor r9d, r9d
    a.e(&[0x4C, 0x8D, 0x45, 0xFF]); // lea r8, [rbp-1]
    a.e(&[0x41, 0xBA, 10, 0, 0, 0]); // mov r10d, 10
    a.e(&[0x48, 0x85, 0xC0]); // test rax, rax
    a.jcc(9, positive);
    a.e(&[0x41, 0xB9, 1, 0, 0, 0]); // mov r9d, 1
    a.e(&[0x48, 0xF7, 0xD8]); // neg rax (the lowest number stays right read unsigned)
    a.bind(positive);
    a.bind(digits);
    a.e(&[0x31, 0xD2]); // xor edx, edx
    a.e(&[0x49, 0xF7, 0xF2]); // div r10
    a.e(&[0x80, 0xC2, 0x30]); // add dl, '0'
    a.e(&[0x41, 0x88, 0x10]); // mov [r8], dl
    a.e(&[0x49, 0xFF, 0xC8]); // dec r8
    a.e(&[0x48, 0x85, 0xC0]); // test rax, rax
    a.jcc(5, digits);
    a.e(&[0x4D, 0x85, 0xC9]); // test r9, r9
    a.jcc(4, sign_done);
    a.e(&[0x41, 0xC6, 0x00, b'-']); // mov byte [r8], '-'
    a.e(&[0x49, 0xFF, 0xC8]); // dec r8
    a.bind(sign_done);
    a.e(&[0x49, 0x8D, 0x48, 0x01]); // lea rcx, [r8+1]
    a.e(&[0x48, 0x89, 0xEA]); // mov rdx, rbp
    a.e(&[0x48, 0x29, 0xCA]); // sub rdx, rcx
    a.call("rt.print_str");
    a.epilogue();
    let print_int = a.finish();

    // rt.start: the process begins here
    let mut a = Asm::new();
    a.e(&[0x48, 0x83, 0xEC, 0x28]); // sub rsp, 40
    a.call(entry);
    a.e(&[0x48, 0x89, 0xC1]); // mov rcx, rax
    a.call_import("ExitProcess");
    a.e(&[0xCC]);
    let start = a.finish();

    vec![("rt.start".into(), start), ("rt.print_str".into(), print_str), ("rt.print_int".into(), print_int)]
}
