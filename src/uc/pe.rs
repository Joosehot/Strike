//! The functions' machine code to a Windows executable (PE32+), written
//! whole by Strike: no linker. Three sections: .text (the code), .rdata
//! (the strings and the import table of KERNEL32) and .bss (zeros the
//! program writes: its globals, the input buffer). The image has a fixed
//! base, so nothing is relocated when it loads.

use super::x64::{Code, Target, IMPORTS};
use std::collections::HashMap;

const BASE: u64 = 0x1_4000_0000;
const SECTION: u32 = 0x1000;
const FILE: u32 = 0x200;
const TEXT_RVA: u32 = SECTION;

fn align(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

fn u16le(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn u32le(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}
fn u64le(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// Lays out the functions (in the order given), fills every distance, and
/// returns the bytes of the executable that starts at `entry`.
pub fn link(funcs: &[(String, Code)], entry: &str, bss_min: u32) -> Result<Vec<u8>, String> {
    let mut text = Vec::new();
    let mut at: HashMap<&str, u32> = HashMap::new();
    for (name, code) in funcs {
        while text.len() % 16 != 0 {
            text.push(0xCC);
        }
        at.insert(name.as_str(), text.len() as u32);
        text.extend_from_slice(&code.bytes);
    }
    let text_len = text.len() as u32;
    let rdata_rva = TEXT_RVA + align(text_len.max(1), SECTION);

    // .rdata: the strings, then the import directory, lookup table, address
    // table, names
    let mut rdata: Vec<u8> = Vec::new();
    let mut strings: HashMap<&[u8], u32> = HashMap::new();
    for (_, code) in funcs {
        for (_, t) in &code.relocs {
            if let Target::Data(s) = t {
                if !strings.contains_key(s.as_slice()) {
                    strings.insert(s.as_slice(), rdata.len() as u32);
                    rdata.extend_from_slice(s);
                }
            }
        }
    }
    while rdata.len() % 8 != 0 {
        rdata.push(0);
    }
    let dir = rdata.len() as u32;
    let n = IMPORTS.len() as u32;
    let ilt = dir + 40;
    let iat = ilt + 8 * (n + 1);
    let mut names_at = iat + 8 * (n + 1);
    let mut hint_names = Vec::new();
    let mut name_rvas = Vec::new();
    for name in IMPORTS {
        name_rvas.push(rdata_rva + names_at);
        let mut e = vec![0u8, 0u8];
        e.extend_from_slice(name.as_bytes());
        e.push(0);
        if e.len() % 2 == 1 {
            e.push(0);
        }
        names_at += e.len() as u32;
        hint_names.extend(e);
    }
    let dll = names_at;
    // the import directory: KERNEL32, then an empty entry
    u32le(&mut rdata, rdata_rva + ilt);
    u32le(&mut rdata, 0);
    u32le(&mut rdata, 0);
    u32le(&mut rdata, rdata_rva + dll);
    u32le(&mut rdata, rdata_rva + iat);
    rdata.extend_from_slice(&[0; 20]);
    for _ in 0..2 {
        for r in &name_rvas {
            u64le(&mut rdata, *r as u64);
        }
        u64le(&mut rdata, 0);
    }
    rdata.extend(hint_names);
    rdata.extend_from_slice(b"KERNEL32.dll\0");
    let rdata_len = rdata.len() as u32;
    // .bss: as far as the furthest place written (8 bytes each), and at
    // least as far as the globals go (an array's elements past its start)
    let bss_len = funcs
        .iter()
        .flat_map(|(_, c)| c.relocs.iter())
        .filter_map(|(_, t)| if let Target::Writable(o) = t { Some(o + 8) } else { None })
        .max()
        .unwrap_or(0)
        .max(bss_min)
        .max(8);
    let bss_rva = rdata_rva + align(rdata_len, SECTION);

    // every distance
    for (name, code) in funcs {
        let base = at[name.as_str()];
        for (field, t) in &code.relocs {
            let target = match t {
                Target::Func(s) => TEXT_RVA + *at.get(s.as_str()).ok_or_else(|| format!("no function {s} to call"))?,
                Target::Import(s) => {
                    let i = IMPORTS.iter().position(|x| x == s).ok_or_else(|| format!("{s} is not imported"))?;
                    rdata_rva + iat + 8 * i as u32
                }
                Target::Data(s) => rdata_rva + strings[s.as_slice()],
                Target::Writable(o) => bss_rva + o,
            };
            let from = TEXT_RVA + base + field + 4;
            let rel = target.wrapping_sub(from) as i32;
            let p = (base + field) as usize;
            text[p..p + 4].copy_from_slice(&rel.to_le_bytes());
        }
    }
    let entry_rva = TEXT_RVA + *at.get(entry).ok_or("no entry")?;

    let text_raw = align(text_len.max(1), FILE);
    let rdata_raw = align(rdata_len, FILE);
    let headers = FILE;
    let image = bss_rva + align(bss_len, SECTION);

    let mut b = Vec::with_capacity((headers + text_raw + rdata_raw) as usize);
    // DOS header: only its magic and where the PE header is
    b.extend_from_slice(b"MZ");
    b.resize(0x3C, 0);
    u32le(&mut b, 0x40);
    b.extend_from_slice(b"PE\0\0");
    // COFF header
    u16le(&mut b, 0x8664);
    u16le(&mut b, 3);
    u32le(&mut b, 0);
    u32le(&mut b, 0);
    u32le(&mut b, 0);
    u16le(&mut b, 240);
    u16le(&mut b, 0x0023); // no relocations, executable, large addresses
    // optional header (PE32+)
    u16le(&mut b, 0x20B);
    b.extend_from_slice(&[1, 0]);
    u32le(&mut b, text_raw);
    u32le(&mut b, rdata_raw);
    u32le(&mut b, align(bss_len, FILE));
    u32le(&mut b, entry_rva);
    u32le(&mut b, TEXT_RVA);
    u64le(&mut b, BASE);
    u32le(&mut b, SECTION);
    u32le(&mut b, FILE);
    for v in [6u16, 0, 0, 0, 6, 0] {
        u16le(&mut b, v);
    }
    u32le(&mut b, 0);
    u32le(&mut b, image);
    u32le(&mut b, headers);
    u32le(&mut b, 0);
    u16le(&mut b, 3); // console
    u16le(&mut b, 0x8100); // NX compatible, terminal server aware
    for v in [0x10_0000u64, 0x1000, 0x10_0000, 0x1000] {
        u64le(&mut b, v);
    }
    u32le(&mut b, 0);
    u32le(&mut b, 16);
    for d in 0..16 {
        let (rva, size) = match d {
            1 => (rdata_rva + dir, 40),
            12 => (rdata_rva + iat, 8 * (n + 1)),
            _ => (0, 0),
        };
        u32le(&mut b, rva);
        u32le(&mut b, size);
    }
    // section headers
    for (name, vsize, rva, raw, ptr, flags) in [
        (b".text\0\0\0", text_len, TEXT_RVA, text_raw, headers, 0x6000_0020u32),
        (b".rdata\0\0", rdata_len, rdata_rva, rdata_raw, headers + text_raw, 0x4000_0040),
        // zeros, read and written, with nothing in the file
        (b".bss\0\0\0\0", bss_len, bss_rva, 0, 0, 0xC000_0080),
    ] {
        b.extend_from_slice(name);
        u32le(&mut b, vsize);
        u32le(&mut b, rva);
        u32le(&mut b, raw);
        u32le(&mut b, ptr);
        u32le(&mut b, 0);
        u32le(&mut b, 0);
        u16le(&mut b, 0);
        u16le(&mut b, 0);
        u32le(&mut b, flags);
    }
    b.resize(headers as usize, 0);
    b.extend_from_slice(&text);
    b.resize((headers + text_raw) as usize, 0);
    b.extend_from_slice(&rdata);
    b.resize((headers + text_raw + rdata_raw) as usize, 0);
    Ok(b)
}
