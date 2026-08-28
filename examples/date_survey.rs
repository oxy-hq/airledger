//! Survey raw Date strings across ledger tabs — how many rows would
//! fail the %Y-%m-%d decode (and so vanish from date-filtered views)?
use airledger_engine::{parse_view, CellValue, SheetsRepository};
use std::collections::BTreeMap;

fn survey(repo: &SheetsRepository, table: &str, date_expr: &str) {
    let yaml = format!(
        "name: survey\ndatasource: gsheets\ntable: \"{table}\"\ndimensions:\n  - {{ name: d, type: string, expr: {date_expr} }}\n"
    );
    let view = parse_view(&yaml).unwrap();
    let rows = repo.list(&view, None).unwrap();
    let mut ok = 0usize;
    let mut bad: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: BTreeMap<String, String> = BTreeMap::new();
    for r in &rows {
        let s = r.get("d").map(|v| v.to_display_string()).unwrap_or_default();
        if chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").is_ok() {
            ok += 1;
        } else {
            let shape: String = s.chars().map(|c| if c.is_ascii_digit() { '9' } else { c }).collect();
            *bad.entry(shape.clone()).or_default() += 1;
            examples.entry(shape).or_insert(s);
        }
    }
    println!("{table}: {} rows, {ok} decode ok, {} bad", rows.len(), rows.len() - ok);
    for (shape, n) in &bad {
        println!("  bad shape {:?} x{} e.g. {:?}", shape, n, examples[shape]);
    }
}

fn main() {
    let sa = std::fs::read_to_string(
        std::env::var("HOME").unwrap() + "/.config/ledger/service-account.json",
    ).unwrap();
    let repo = SheetsRepository::new(
        "1C1rSudguUv00gYsb7i82XV6OM1V2KSZ4BGwMliwKDG4".into(), &sa,
    ).unwrap();
    survey(&repo, "strength", "Date");
    survey(&repo, "weight", "date");
    survey(&repo, "meals", "eaten_at");
    survey(&repo, "4x4", "Date");
}
