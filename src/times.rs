//! Where Strike's own time goes (STRIKE_TIMES=1). Work done on several
//! threads at once is summed, so those totals can pass the wall time.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

static TIMES: Mutex<BTreeMap<&'static str, (f64, u32, f64)>> = Mutex::new(BTreeMap::new());

pub fn on() -> bool {
    std::env::var_os("STRIKE_TIMES").is_some()
}

/// Times `f` under `name`.
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    let mut m = TIMES.lock().unwrap();
    let s = t.elapsed().as_secs_f64();
    let e = m.entry(name).or_insert((0.0, 0, 0.0));
    e.0 += s;
    e.1 += 1;
    e.2 = e.2.max(s);
    out
}

/// Adds seconds measured elsewhere under `name`.
pub fn add(name: &'static str, secs: f64) {
    let mut m = TIMES.lock().unwrap();
    let e = m.entry(name).or_insert((0.0, 0, 0.0));
    e.0 += secs;
    e.1 += 1;
    e.2 = e.2.max(secs);
}

pub fn report() {
    if !on() {
        return;
    }
    let m = TIMES.lock().unwrap();
    let mut rows: Vec<_> = m.iter().collect();
    rows.sort_by(|a, b| b.1 .0.partial_cmp(&a.1 .0).unwrap());
    eprintln!("strike times (sum of all runs; the longest single run):");
    for (name, (s, n, longest)) in rows {
        eprintln!("  {name:<28} sum {s:>7.3} s  x{n:<3} longest {longest:>6.3} s");
    }
}
