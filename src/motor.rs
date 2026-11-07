//! HotMotor: the rules offer variants, the beam keeps the best partial
//! builds, the judges score the choices together. Every number comes from
//! rules.toml.

use std::collections::BTreeMap;

pub const AXES: [&str; 3] = ["build", "run", "disk"];

#[derive(Clone, Debug)]
pub struct Variant {
    pub value: String,
    pub axes: [f64; 3],
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub name: String,
    pub weight: f64,
    pub variants: Vec<Variant>,
}

#[derive(Debug)]
pub struct Rules {
    pub beam: usize,
    pub profiles: BTreeMap<String, [f64; 3]>,
    pub rules: Vec<Rule>,
    pub judge_lightning: f64,
    pub judge_incremental_one_unit: f64,
}

pub const DEFAULT_RULES: &str = include_str!("../rules.toml");

fn num(t: &toml::Table, key: &str, at: &str) -> Result<f64, String> {
    match t.get(key) {
        Some(toml::Value::Float(f)) => Ok(*f),
        Some(toml::Value::Integer(i)) => Ok(*i as f64),
        _ => Err(format!("rules.toml: {at} needs the number {key}")),
    }
}

fn axes(t: &toml::Table, at: &str) -> Result<[f64; 3], String> {
    let mut a = [0.0; 3];
    for (i, name) in AXES.iter().enumerate() {
        a[i] = num(t, name, at)?;
    }
    Ok(a)
}

impl Rules {
    /// Read and check rules.toml: a typo fails here, not in a build.
    pub fn parse(text: &str) -> Result<Rules, String> {
        let doc: toml::Table = text.parse().map_err(|e| format!("rules.toml: {e}"))?;
        let beam = num(&doc, "beam", "the top")? as usize;
        if beam == 0 {
            return Err("rules.toml: beam must be at least 1".into());
        }
        let mut profiles = BTreeMap::new();
        for (name, p) in doc.get("profile").and_then(|p| p.as_table()).ok_or("rules.toml: no [profile]")? {
            let p = p.as_table().ok_or("rules.toml: a profile is a table")?;
            profiles.insert(name.clone(), axes(p, &format!("profile.{name}"))?);
        }
        // The decisions Strike knows how to turn into rustc flags.
        let known = ["opt_level", "codegen_units", "incremental", "debuginfo"];
        let mut rules = Vec::new();
        for name in known {
            let at = format!("rule.{name}");
            let r = doc
                .get("rule")
                .and_then(|r| r.get(name))
                .and_then(|r| r.as_table())
                .ok_or(format!("rules.toml: no [{at}]"))?;
            let mut variants = Vec::new();
            for v in r.get("variants").and_then(|v| v.as_array()).ok_or(format!("rules.toml: {at} has no variants"))? {
                let v = v.as_table().ok_or(format!("rules.toml: a variant of {at} is not a table"))?;
                let value = v.get("value").and_then(|x| x.as_str()).ok_or(format!("rules.toml: a variant of {at} has no value"))?;
                variants.push(Variant { value: value.to_string(), axes: axes(v, &at)? });
            }
            if variants.is_empty() {
                return Err(format!("rules.toml: {at} has no variants"));
            }
            rules.push(Rule { name: name.to_string(), weight: num(r, "weight", &at)?, variants });
        }
        if let Some(r) = doc.get("rule").and_then(|r| r.as_table()) {
            for name in r.keys() {
                if !known.contains(&name.as_str()) {
                    return Err(format!("rules.toml: unknown rule {name} (known: {})", known.join(", ")));
                }
            }
        }
        let judge = doc.get("judge").and_then(|j| j.as_table()).ok_or("rules.toml: no [judge]")?;
        Ok(Rules {
            beam,
            profiles,
            rules,
            judge_lightning: num(judge, "lightning", "judge")?,
            judge_incremental_one_unit: num(judge, "incremental_one_unit", "judge")?,
        })
    }
}

/// A choice of one variant per rule (so far): rule name -> value.
pub type Choice = Vec<(String, String)>;

/// The rustc flags of a complete choice.
pub fn flags(choice: &Choice) -> Vec<String> {
    let get = |n: &str| choice.iter().find(|(r, _)| r == n).map(|(_, v)| v.as_str());
    let mut f = Vec::new();
    if let Some(v) = get("opt_level") {
        f.push(format!("-Copt-level={v}"));
    }
    if let Some(v) = get("codegen_units") {
        f.push(format!("-Ccodegen-units={v}"));
    }
    if let Some(v) = get("debuginfo") {
        f.push(format!("-Cdebuginfo={v}"));
    }
    if get("incremental") == Some("on") {
        // The directory is not part of the flags: it does not change the output.
        f.push("incremental".to_string());
    }
    f
}

/// What the lightning knows for the judges: the flag sets whose builds are
/// branded for every unit, and for how many units each one is.
pub struct Branded {
    pub sets: Vec<(Vec<String>, usize)>,
    pub units: usize,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub choice: Choice,
    pub local: f64,
    pub judged: f64,
    pub notes: Vec<String>,
}

impl Candidate {
    pub fn score(&self) -> f64 {
        self.local + self.judged
    }
}

fn judge(rules: &Rules, choice: &Choice, branded: &Branded, profile: &[f64; 3]) -> (f64, Vec<String>) {
    let mut total = 0.0;
    let mut notes = Vec::new();
    let get = |n: &str| choice.iter().find(|(r, _)| r == n).map(|(_, v)| v.as_str());

    if get("incremental") == Some("on") && get("codegen_units") == Some("1") {
        total -= rules.judge_incremental_one_unit;
        notes.push(format!("-{} incremental keeps nothing with one codegen unit", rules.judge_incremental_one_unit));
    }

    // The lightning judge: the share of units whose build with choices that
    // agree with this one so far is already branded. A brand saves build
    // time only, so it counts as much as the profile asks for build time.
    if branded.units > 0 {
        let partial = flags(choice);
        let best = branded
            .sets
            .iter()
            .filter(|(set, _)| agrees(&partial, choice, set))
            .map(|(_, n)| *n)
            .max()
            .unwrap_or(0);
        if best > 0 {
            let bonus = rules.judge_lightning * profile[0] * best as f64 / branded.units as f64;
            total += bonus;
            notes.push(format!("+{bonus:.2} lightning: {best} of {} units already branded", branded.units));
        }
    }
    (total, notes)
}

/// Does a branded flag set agree with the choices made so far?
fn agrees(partial: &[String], choice: &Choice, set: &[String]) -> bool {
    let chose_incremental = choice.iter().any(|(r, _)| r == "incremental");
    partial.iter().all(|f| set.contains(f))
        && (!chose_incremental || partial.contains(&"incremental".to_string()) == set.contains(&"incremental".to_string()))
}

/// The beam search: rule by rule, every partial build grows by every variant
/// and the best `beam` are kept. Ties break on the variants' order in
/// rules.toml, so the result is deterministic. Returns the finalists, best
/// first.
pub fn search(rules: &Rules, profile: &[f64; 3], branded: &Branded) -> Vec<Candidate> {
    let mut beam = vec![Candidate { choice: Vec::new(), local: 0.0, judged: 0.0, notes: Vec::new() }];
    for rule in &rules.rules {
        let mut grown = Vec::new();
        for c in &beam {
            for v in &rule.variants {
                let mut choice = c.choice.clone();
                choice.push((rule.name.clone(), v.value.clone()));
                let dot: f64 = v.axes.iter().zip(profile).map(|(a, p)| a * p).sum();
                let local = c.local + rule.weight * dot;
                let (judged, notes) = judge(rules, &choice, branded, profile);
                grown.push(Candidate { choice, local, judged, notes });
            }
        }
        // A stable sort keeps the earlier variant first on a tie.
        grown.sort_by(|a, b| b.score().partial_cmp(&a.score()).unwrap_or(std::cmp::Ordering::Equal));
        grown.truncate(rules.beam);
        beam = grown;
    }
    beam
}
