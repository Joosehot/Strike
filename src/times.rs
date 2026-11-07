//! Where Strike's own time goes (STRIKE_TIMES=1). Work done on several
//! threads at once is summed, so those totals can pass the wall time.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

static TIMES: Mutex<BTreeMap<&'static str, (f64, u32)>> = Mutex::new(BTreeMap::new());

pub fn on() -> bool {
    std::env::var_os("STRIKE_TIMES").is_some()
}

/// Times `f` under `name`.
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    let mut m = TIMES.lock().unwrap();
    let e = m.entry(name).or_insert((0.0, 0));
    e.0 += t.elapsed().as_secs_f64();
    e.1 += 1;
    out
}

/// Adds seconds measured elsewhere under `name`.
pub fn add(name: &'static str, secs: f64) {
    let mut m = TIMES.lock().unwrap();
    let e = m.entry(name).or_insert((0.0, 0));
    e.0 += secs;
    e.1 += 1;
}

pub fn report() {
    if !on() {
        return;
    }
    let m = TIMES.lock().unwrap();
    let mut rows: Vec<_> = m.iter().collect();
    rows.sort_by(|a, b| b.1 .0.partial_cmp(&a.1 .0).unwrap());
    eprintln!("strike times:");
    for (name, (s, n)) in rows {
        eprintln!("  {name:<28} {s:>8.3} s  x{n}");
    }
}
