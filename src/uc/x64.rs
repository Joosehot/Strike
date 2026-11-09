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
    /// an offset in the zeros the program writes (.bss)
    Writable(u32),
}

/// A function's machine code, and the 32-bit fields still to be filled with
/// the distance to their target (each measured from the field's end).
#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub bytes: Vec<u8>,
    pub relocs: Vec<(u32, Target)>,
}

pub const IMPORTS: [&str; 5] = ["GetStdHandle", "WriteFile", "ExitProcess", "ReadFile", "VirtualAlloc"];

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
    // the slots below rbp, then the function's own memory below them
    let memory = -(f.slots as i32 * 8 + (f.frame as i32 + 7) / 8 * 8);
    let frame = (-memory + 15) / 16 * 16;
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
    // where checked operations go when a number would not fit
    let mut trap_at: Option<u32> = None;
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
            Op::Sext(n) => {
                a.e(&[0x58]);
                sext_reg(&mut a, 0, *n);
                a.e(&[0x50]);
            }
            Op::LoadG(g) => {
                a.e(&[0x48, 0x8B, 0x05]); // mov rax, [global]
                a.reloc(global(*g));
                a.e(&[0x50]);
                depth += 1;
            }
            Op::StoreG(g) => {
                a.e(&[0x58, 0x48, 0x89, 0x05]); // pop rax; mov [global], rax
                a.reloc(global(*g));
                depth -= 1;
            }
            Op::ReadToken => {
                call(&mut a, "rt.read_token", 0, &mut depth, false);
                a.e(&[0x50, 0x52]); // push rax (the number); push rdx (whether there was one)
                depth += 2;
            }
            Op::ReadLine => {
                call(&mut a, "rt.read_line", 0, &mut depth, true);
            }
            Op::LoadM(w) => {
                a.e(&[0x58]);
                a.e(match w {
                    1 => &[0x48, 0x0F, 0xBE, 0x00], // movsx rax, byte [rax]
                    2 => &[0x48, 0x0F, 0xBF, 0x00],
                    4 => &[0x48, 0x63, 0x00], // movsxd rax, dword [rax]
                    _ => &[0x48, 0x8B, 0x00],
                });
                a.e(&[0x50]);
            }
            Op::StoreM(w) => {
                a.e(&[0x59, 0x58]); // pop rcx (the value); pop rax (the address)
                a.e(match w {
                    1 => &[0x88, 0x08], // mov [rax], cl
                    2 => &[0x66, 0x89, 0x08],
                    4 => &[0x89, 0x08],
                    _ => &[0x48, 0x89, 0x08],
                });
                depth -= 2;
            }
            Op::Frame(n) => {
                a.e(&[0x48, 0x8D, 0x85]); // lea rax, [rbp + d]
                a.d32(memory + *n as i32);
                a.e(&[0x50]);
                depth += 1;
            }
            Op::AddrG(g) => {
                a.e(&[0x48, 0x8D, 0x05]); // lea rax, [global]
                a.reloc(global(*g));
                a.e(&[0x50]);
                depth += 1;
            }
            Op::Fail(code, m) => {
                fail_here(&mut a, *code, m);
            }
            Op::Un(u) => {
                a.e(&[0x58]);
                match u {
                    UnOp::Neg => a.e(&[0x48, 0xF7, 0xD8]),
                    UnOp::Bnot => a.e(&[0x48, 0xF7, 0xD0]),
                    UnOp::Not => a.e(&[0x48, 0x85, 0xC0, 0x0F, 0x94, 0xC0, 0x0F, 0xB6, 0xC0]),
                    UnOp::Plus => {}
                    UnOp::NegOv => {
                        let t = *trap_at.get_or_insert_with(|| a.label());
                        checked(&mut a, None, t);
                    }
                }
                a.e(&[0x50]);
            }
            Op::Bin(b) => {
                a.e(&[0x59, 0x58]); // pop rcx (right); pop rax (left)
                let set = |a: &mut Asm, cc: u8| a.e(&[0x48, 0x39, 0xC8, 0x0F, cc, 0xC0, 0x0F, 0xB6, 0xC0]);
                match b {
                    BinOp::AddOv | BinOp::SubOv | BinOp::MulOv | BinOp::ShlOv => {
                        let t = *trap_at.get_or_insert_with(|| a.label());
                        checked(&mut a, Some(*b), t);
                    }
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
                    BinOp::Ult => set(&mut a, 0x92),
                    BinOp::Ule => set(&mut a, 0x96),
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
    if let Some(t) = trap_at {
        trap(&mut a, t);
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

    vec![
        ("rt.start".into(), start),
        ("rt.print_str".into(), print_str),
        ("rt.print_int".into(), print_int),
        ("rt.overflow".into(), overflow()),
        ("rt.fail".into(), fail()),
        ("rt.getc".into(), getc()),
        ("rt.read_token".into(), read_token()),
        ("rt.read_line".into(), read_line()),
        (super::ir::ALLOC.into(), alloc()),
    ]
}

/// movsx r, r (the low n bits of r, sign-extended).
pub(super) fn sext_reg(a: &mut Asm, r: u8, n: u8) {
    let rex = 0x48 | ((r >> 3) & 1) << 2 | ((r >> 3) & 1);
    let modrm = 0xC0 | (r & 7) << 3 | (r & 7);
    match n {
        32 => a.e(&[rex, 0x63, modrm]),
        16 => a.e(&[rex, 0x0F, 0xBF, modrm]),
        8 => a.e(&[rex, 0x0F, 0xBE, modrm]),
        _ => unreachable!("widths are 8, 16, 32"),
    }
}

/// rax op rcx for the checked operations (or neg rax), jumping to `trap`
/// where the number would not fit 64 bits. rdx and r8 may be used.
pub(super) fn checked(a: &mut Asm, op: Option<BinOp>, trap: u32) {
    const JO: u8 = 0;
    match op {
        None => {
            a.e(&[0x48, 0xF7, 0xD8]); // neg rax
            a.jcc(JO, trap);
        }
        Some(BinOp::AddOv) => {
            a.e(&[0x48, 0x01, 0xC8]);
            a.jcc(JO, trap);
        }
        Some(BinOp::SubOv) => {
            a.e(&[0x48, 0x29, 0xC8]);
            a.jcc(JO, trap);
        }
        Some(BinOp::MulOv) => {
            a.e(&[0x48, 0x0F, 0xAF, 0xC1]);
            a.jcc(JO, trap);
        }
        Some(BinOp::ShlOv) => {
            // a count of 0..63 whose shift can be undone exactly
            a.e(&[0x48, 0x83, 0xF9, 0x3F]); // cmp rcx, 63
            a.jcc(7, trap); // above, unsigned: also a negative count
            a.e(&[0x48, 0x89, 0xC2, 0x48, 0xD3, 0xE0]); // mov rdx, rax; shl rax, cl
            a.e(&[0x49, 0x89, 0xC0, 0x49, 0xD3, 0xF8, 0x49, 0x39, 0xD0]); // mov r8, rax; sar r8, cl; cmp r8, rdx
            a.jcc(5, trap);
        }
        Some(b) => unreachable!("{b:?} is not checked"),
    }
}

/// Where a function's checked operations jump: the stack aligned, and
/// rt.overflow, which does not come back.
pub(super) fn trap(a: &mut Asm, at: u32) {
    a.bind(at);
    a.e(&[0x48, 0x83, 0xE4, 0xF0, 0x48, 0x83, 0xEC, 0x20]); // and rsp, -16; sub rsp, 32
    a.call("rt.overflow");
    a.e(&[0xCC]);
}

/// What rt.overflow says, on standard error, before the program stops
/// with exit code 1.
pub const OVERFLOW: &[u8] = b"strike: a number grew past 64 bits here; this language's numbers do not stop there, and Strike has no larger numbers yet, so the program stops\n";
const NOT_A_NUMBER: &[u8] = b"strike: the line read is not a whole number, so the program stops (Python's int() would raise ValueError)\n";
const NO_LINE: &[u8] = b"strike: there was no line to read, so the program stops (Python's input() would raise EOFError)\n";

/// The zeros the program writes (.bss): the input buffer, where in it the
/// next byte is and how many it holds, then the globals.
const BUF: u32 = 0;
const BUF_SIZE: i32 = 4096;
const POS: u32 = 4096;
const LEN: u32 = 4104;
/// the allocator's next free byte and the end of its chunk
const HEAP: u32 = 4112;
const HEAP_END: u32 = 4120;
pub const GLOBALS: u32 = 4128;

/// The place of global g in the zeros.
pub fn global(g: u32) -> Target {
    Target::Writable(GLOBALS + 8 * g)
}

/// rt.fail(rcx = message, rdx = its length, r8 = exit code): the message
/// on standard error, then the program stops with that code.
fn fail() -> Code {
    let mut a = Asm::new();
    a.e(&[0x48, 0x83, 0xEC, 0x48]); // sub rsp, 72
    a.e(&[0x48, 0x89, 0x4C, 0x24, 0x28]); // mov [rsp+40], rcx
    a.e(&[0x48, 0x89, 0x54, 0x24, 0x30]); // mov [rsp+48], rdx
    a.e(&[0x4C, 0x89, 0x44, 0x24, 0x38]); // mov [rsp+56], r8
    a.e(&[0x48, 0xC7, 0xC1, 0xF4, 0xFF, 0xFF, 0xFF]); // mov rcx, -12 (standard error)
    a.call_import("GetStdHandle");
    a.e(&[0x48, 0x89, 0xC1]); // mov rcx, rax
    a.e(&[0x48, 0x8B, 0x54, 0x24, 0x28]); // mov rdx, [rsp+40]
    a.e(&[0x4C, 0x8B, 0x44, 0x24, 0x30]); // mov r8, [rsp+48]
    a.e(&[0x4C, 0x8D, 0x4C, 0x24, 0x40]); // lea r9, [rsp+64]
    a.e(&[0x48, 0xC7, 0x44, 0x24, 0x20, 0, 0, 0, 0]); // mov qword [rsp+32], 0
    a.call_import("WriteFile");
    a.e(&[0x8B, 0x4C, 0x24, 0x38]); // mov ecx, [rsp+56]
    a.call_import("ExitProcess");
    a.e(&[0xCC]);
    a.finish()
}

/// Calls rt.fail with a message and exit code 1 (the stack aligned for a
/// call).
fn fail_with(a: &mut Asm, msg: &[u8]) {
    a.e(&[0x48, 0x8D, 0x0D]); // lea rcx, [rip + msg]
    a.reloc(Target::Data(msg.to_vec()));
    a.e(&[0x48, 0xC7, 0xC2]); // mov rdx, len
    a.d32(msg.len() as i32);
    a.e(&[0x41, 0xB8, 1, 0, 0, 0]); // mov r8d, 1
    a.call("rt.fail");
}

/// Op::Fail wherever the stack is: aligned, then rt.fail, which does not
/// come back.
pub(super) fn fail_here(a: &mut Asm, code: u8, msg: &[u8]) {
    a.e(&[0x48, 0x83, 0xE4, 0xF0, 0x48, 0x83, 0xEC, 0x20]); // and rsp, -16; sub rsp, 32
    a.e(&[0x48, 0x8D, 0x0D]); // lea rcx, [rip + msg]
    a.reloc(Target::Data(msg.to_vec()));
    a.e(&[0x48, 0xC7, 0xC2]); // mov rdx, len
    a.d32(msg.len() as i32);
    a.e(&[0x41, 0xB8, code, 0, 0, 0]); // mov r8d, code
    a.call("rt.fail");
    a.e(&[0xCC]);
}

const OUT_OF_MEMORY: &[u8] = b"strike: out of memory (or a size that is not one), so the program stops\n";
/// what the allocator takes from Windows at a time
const CHUNK: i32 = 64 << 20;

/// rt.alloc(rcx = bytes): the address of that many new zero bytes, 16-aligned.
/// Bump allocation in chunks Windows gives zeroed (VirtualAlloc); nothing
/// is given back in this stage.
fn alloc() -> Code {
    let mut a = Asm::new();
    let (more, bad) = (a.label(), a.label());
    a.e(&[0x48, 0x85, 0xC9]); // test rcx, rcx
    a.jcc(8, bad); // js: a negative size
    a.e(&[0x48, 0x83, 0xC1, 0x0F, 0x48, 0x83, 0xE1, 0xF0]); // add rcx, 15; and rcx, -16
    a.jcc(8, bad); // past 2^63
    a.e(&[0x48, 0x8B, 0x05]); // mov rax, [heap]
    a.reloc(Target::Writable(HEAP));
    a.e(&[0x48, 0x8D, 0x14, 0x08]); // lea rdx, [rax + rcx]
    a.e(&[0x48, 0x3B, 0x15]); // cmp rdx, [heap_end]
    a.reloc(Target::Writable(HEAP_END));
    a.jcc(7, more); // ja
    a.e(&[0x48, 0x89, 0x15]); // mov [heap], rdx
    a.reloc(Target::Writable(HEAP));
    a.e(&[0xC3]);
    a.bind(more);
    a.e(&[0x48, 0x83, 0xEC, 0x38]); // sub rsp, 56
    a.e(&[0x48, 0x89, 0x4C, 0x24, 0x28]); // mov [rsp+40], rcx
    a.e(&[0x48, 0xC7, 0xC2]); // mov rdx, CHUNK
    a.d32(CHUNK);
    a.e(&[0x48, 0x39, 0xD1, 0x48, 0x0F, 0x47, 0xD1]); // cmp rcx, rdx; cmova rdx, rcx
    a.e(&[0x48, 0x89, 0x54, 0x24, 0x30]); // mov [rsp+48], rdx
    a.e(&[0x31, 0xC9]); // xor ecx, ecx
    a.e(&[0x41, 0xB8, 0x00, 0x30, 0, 0, 0x41, 0xB9, 4, 0, 0, 0]); // mov r8d, MEM_COMMIT | MEM_RESERVE; mov r9d, PAGE_READWRITE
    a.call_import("VirtualAlloc");
    a.e(&[0x48, 0x85, 0xC0]); // test rax, rax
    a.jcc(4, bad);
    a.e(&[0x48, 0x8B, 0x4C, 0x24, 0x28, 0x48, 0x8D, 0x14, 0x08]); // mov rcx, [rsp+40]; lea rdx, [rax + rcx]
    a.e(&[0x48, 0x89, 0x15]); // mov [heap], rdx
    a.reloc(Target::Writable(HEAP));
    a.e(&[0x48, 0x8B, 0x54, 0x24, 0x30, 0x48, 0x01, 0xC2]); // mov rdx, [rsp+48]; add rdx, rax
    a.e(&[0x48, 0x89, 0x15]); // mov [heap_end], rdx
    a.reloc(Target::Writable(HEAP_END));
    a.e(&[0x48, 0x83, 0xC4, 0x38, 0xC3]); // add rsp, 56; ret
    a.bind(bad);
    fail_here(&mut a, 1, OUT_OF_MEMORY);
    a.finish()
}

fn overflow() -> Code {
    let mut a = Asm::new();
    a.e(&[0x48, 0x83, 0xEC, 0x28]); // sub rsp, 40
    fail_with(&mut a, OVERFLOW);
    a.e(&[0xCC]);
    a.finish()
}

/// rt.getc: the next byte of standard input in rax, or -1 at its end. The
/// input is read 4096 bytes at a time.
fn getc() -> Code {
    let mut a = Asm::new();
    let (have, end) = (a.label(), a.label());
    a.e(&[0x48, 0x83, 0xEC, 0x38]); // sub rsp, 56
    a.e(&[0x48, 0x8B, 0x05]); // mov rax, [pos]
    a.reloc(Target::Writable(POS));
    a.e(&[0x48, 0x3B, 0x05]); // cmp rax, [len]
    a.reloc(Target::Writable(LEN));
    a.jcc(0xC, have); // jl
    // empty: read more
    a.e(&[0x31, 0xC0, 0x48, 0x89, 0x05]); // xor eax, eax; mov [len], rax
    a.reloc(Target::Writable(LEN));
    a.e(&[0x48, 0xC7, 0xC1, 0xF6, 0xFF, 0xFF, 0xFF]); // mov rcx, -10 (standard input)
    a.call_import("GetStdHandle");
    a.e(&[0x48, 0x89, 0xC1]); // mov rcx, rax
    a.e(&[0x48, 0x8D, 0x15]); // lea rdx, [buf]
    a.reloc(Target::Writable(BUF));
    a.e(&[0x49, 0xC7, 0xC0]); // mov r8, 4096
    a.d32(BUF_SIZE);
    a.e(&[0x4C, 0x8D, 0x0D]); // lea r9, [len] (ReadFile writes 32 bits of it)
    a.reloc(Target::Writable(LEN));
    a.e(&[0x48, 0xC7, 0x44, 0x24, 0x20, 0, 0, 0, 0]); // mov qword [rsp+32], 0
    a.call_import("ReadFile");
    a.e(&[0x31, 0xC0, 0x48, 0x89, 0x05]); // xor eax, eax; mov [pos], rax
    a.reloc(Target::Writable(POS));
    a.e(&[0x48, 0x3B, 0x05]); // cmp rax, [len]
    a.reloc(Target::Writable(LEN));
    a.jcc(0xC, have);
    a.e(&[0x48, 0xC7, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]); // mov rax, -1
    a.jmp(end);
    a.bind(have);
    a.e(&[0x48, 0x8D, 0x0D]); // lea rcx, [buf]
    a.reloc(Target::Writable(BUF));
    a.e(&[0x0F, 0xB6, 0x04, 0x01]); // movzx eax, byte [rcx + rax]
    a.e(&[0x48, 0xFF, 0x05]); // inc qword [pos]
    a.reloc(Target::Writable(POS));
    a.bind(end);
    a.e(&[0x48, 0x83, 0xC4, 0x38, 0xC3]); // add rsp, 56; ret
    a.finish()
}

/// Gives back the byte rt.getc returned last (not at the input's end).
fn unget(a: &mut Asm) {
    a.e(&[0x48, 0xFF, 0x0D]); // dec qword [pos]
    a.reloc(Target::Writable(POS));
}

/// rt.read_token: blanks skipped, then an optional sign and digits.
/// rax = the number and rdx = 1, or rdx = 0 where there is none (the byte
/// that is not a digit stays to be read, as C's scanf leaves it).
fn read_token() -> Code {
    let mut a = Asm::new();
    let (skip, not_minus, first, digits, done, positive, fail_unget, fail, out) =
        (a.label(), a.label(), a.label(), a.label(), a.label(), a.label(), a.label(), a.label(), a.label());
    a.e(&[0x53, 0x56, 0x57, 0x48, 0x83, 0xEC, 0x20]); // push rbx, rsi, rdi; sub rsp, 32
    a.bind(skip);
    a.call("rt.getc");
    a.e(&[0x48, 0x83, 0xF8, 0xFF]); // cmp rax, -1
    a.jcc(4, fail);
    for blank in [32u8, 9, 10, 13, 11, 12] {
        a.e(&[0x48, 0x83, 0xF8, blank]);
        a.jcc(4, skip);
    }
    a.e(&[0x31, 0xF6]); // xor esi, esi (not negative)
    a.e(&[0x48, 0x83, 0xF8, b'-']);
    a.jcc(5, not_minus);
    a.e(&[0xBE, 1, 0, 0, 0]); // mov esi, 1
    a.call("rt.getc");
    a.jmp(first);
    a.bind(not_minus);
    a.e(&[0x48, 0x83, 0xF8, b'+']);
    a.jcc(5, first);
    a.call("rt.getc");
    a.bind(first);
    a.e(&[0x48, 0x89, 0xC7, 0x48, 0x83, 0xEF, 0x30, 0x48, 0x83, 0xFF, 0x09]); // rdi = rax - '0'; cmp rdi, 9
    a.jcc(7, fail_unget); // above, unsigned: not a digit
    a.e(&[0x31, 0xDB]); // xor ebx, ebx
    a.bind(digits);
    a.e(&[0x48, 0x6B, 0xDB, 0x0A, 0x48, 0x01, 0xFB]); // rbx = rbx * 10 + rdi
    a.call("rt.getc");
    a.e(&[0x48, 0x89, 0xC7, 0x48, 0x83, 0xEF, 0x30, 0x48, 0x83, 0xFF, 0x09]);
    a.jcc(6, digits); // below or equal, unsigned: a digit
    a.e(&[0x48, 0x83, 0xF8, 0xFF]);
    a.jcc(4, done);
    unget(&mut a);
    a.bind(done);
    a.e(&[0x48, 0x89, 0xD8, 0x48, 0x85, 0xF6]); // mov rax, rbx; test rsi, rsi
    a.jcc(4, positive);
    a.e(&[0x48, 0xF7, 0xD8]); // neg rax
    a.bind(positive);
    a.e(&[0xBA, 1, 0, 0, 0]); // mov edx, 1
    a.jmp(out);
    a.bind(fail_unget);
    a.e(&[0x48, 0x83, 0xF8, 0xFF]);
    a.jcc(4, fail);
    unget(&mut a);
    a.bind(fail);
    a.e(&[0x31, 0xC0, 0x31, 0xD2]); // xor eax, eax; xor edx, edx
    a.bind(out);
    a.e(&[0x48, 0x83, 0xC4, 0x20, 0x5F, 0x5E, 0x5B, 0xC3]); // add rsp, 32; pop rdi, rsi, rbx; ret
    a.finish()
}

/// rt.read_line: one line, which must be a whole number with blanks
/// around it at most, as Python's int(input()) takes it; anything else
/// stops the program with Python's reason, and a number past 64 bits with
/// the overflow's.
fn read_line() -> Code {
    let mut a = Asm::new();
    let (next, ws, read, not_minus, digit, end_line, positive, bad, no_line, too_big) =
        (a.label(), a.label(), a.label(), a.label(), a.label(), a.label(), a.label(), a.label(), a.label(), a.label());
    // rbx the number, rsi negative, rdi how many digits, r12 where: 0
    // before it, 1 in it, 2 after it
    a.e(&[0x53, 0x56, 0x57, 0x41, 0x54, 0x48, 0x83, 0xEC, 0x28]); // push rbx, rsi, rdi, r12; sub rsp, 40
    a.e(&[0x31, 0xDB, 0x31, 0xF6, 0x31, 0xFF, 0x45, 0x31, 0xE4]);
    a.call("rt.getc");
    a.e(&[0x48, 0x83, 0xF8, 0xFF]);
    a.jcc(4, no_line);
    a.bind(next);
    a.e(&[0x48, 0x83, 0xF8, 0x0A]); // cmp rax, '\n'
    a.jcc(4, end_line);
    a.e(&[0x48, 0x83, 0xF8, 0xFF]);
    a.jcc(4, end_line);
    for blank in [32u8, 9, 13, 11, 12] {
        a.e(&[0x48, 0x83, 0xF8, blank]);
        a.jcc(4, ws);
    }
    a.e(&[0x49, 0x83, 0xFC, 0x02]); // cmp r12, 2: something after the number
    a.jcc(4, bad);
    a.e(&[0x48, 0x83, 0xF8, b'-']);
    a.jcc(5, not_minus);
    a.e(&[0x49, 0x83, 0xFC, 0x00]); // a sign only before it
    a.jcc(5, bad);
    a.e(&[0xBE, 1, 0, 0, 0, 0x41, 0xBC, 1, 0, 0, 0]); // mov esi, 1; mov r12d, 1
    a.jmp(read);
    a.bind(not_minus);
    a.e(&[0x48, 0x83, 0xF8, b'+']);
    a.jcc(5, digit);
    a.e(&[0x49, 0x83, 0xFC, 0x00]);
    a.jcc(5, bad);
    a.e(&[0x41, 0xBC, 1, 0, 0, 0]);
    a.jmp(read);
    a.bind(digit);
    a.e(&[0x48, 0x89, 0xC1, 0x48, 0x83, 0xE9, 0x30, 0x48, 0x83, 0xF9, 0x09]); // rcx = rax - '0'; cmp rcx, 9
    a.jcc(7, bad);
    a.e(&[0x48, 0x6B, 0xDB, 0x0A]); // imul rbx, rbx, 10
    a.jcc(0, too_big);
    a.e(&[0x48, 0x01, 0xCB]); // add rbx, rcx
    a.jcc(0, too_big);
    a.e(&[0x48, 0xFF, 0xC7, 0x41, 0xBC, 1, 0, 0, 0]); // inc rdi; mov r12d, 1
    a.jmp(read);
    a.bind(ws);
    a.e(&[0x49, 0x83, 0xFC, 0x01]); // blanks after the number end it
    a.jcc(5, read);
    a.e(&[0x41, 0xBC, 2, 0, 0, 0]);
    a.bind(read);
    a.call("rt.getc");
    a.jmp(next);
    a.bind(end_line);
    a.e(&[0x48, 0x85, 0xFF]); // test rdi, rdi: no digit
    a.jcc(4, bad);
    a.e(&[0x48, 0x89, 0xD8, 0x48, 0x85, 0xF6]); // mov rax, rbx; test rsi, rsi
    a.jcc(4, positive);
    a.e(&[0x48, 0xF7, 0xD8]);
    a.bind(positive);
    a.e(&[0x48, 0x83, 0xC4, 0x28, 0x41, 0x5C, 0x5F, 0x5E, 0x5B, 0xC3]); // add rsp, 40; pop r12, rdi, rsi, rbx; ret
    a.bind(bad);
    fail_with(&mut a, NOT_A_NUMBER);
    a.bind(no_line);
    fail_with(&mut a, NO_LINE);
    a.bind(too_big);
    a.call("rt.overflow");
    a.e(&[0xCC]);
    a.finish()
}

/// What a baking run prints after the program's own output, before the
/// number the program returned.
pub const RETURNED: &[u8] = b"\n\x01strike returned\x01 ";

/// The entry of a baking run (--release): the program, then RETURNED and
/// the number it returned, then exit 0. A run that crashes never prints
/// RETURNED, so a crash is never taken for a result.
pub fn bake_start(entry: &str) -> Code {
    let mut a = Asm::new();
    a.e(&[0x48, 0x83, 0xEC, 0x28]); // sub rsp, 40
    a.call(entry);
    a.e(&[0x48, 0x89, 0x44, 0x24, 0x20]); // mov [rsp+32], rax
    a.e(&[0x48, 0x8D, 0x0D]); // lea rcx, [rip + RETURNED]
    a.reloc(Target::Data(RETURNED.to_vec()));
    a.e(&[0x48, 0xC7, 0xC2]); // mov rdx, len
    a.d32(RETURNED.len() as i32);
    a.call("rt.print_str");
    a.e(&[0x48, 0x8B, 0x4C, 0x24, 0x20]); // mov rcx, [rsp+32]
    a.call("rt.print_int");
    a.e(&[0x31, 0xC9]); // xor ecx, ecx
    a.call_import("ExitProcess");
    a.e(&[0xCC]);
    a.finish()
}
