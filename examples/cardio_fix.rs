//! One-off: Aug 27 cardio rows treadmill -> outdoor running.
//! Speed/incline are already blank on both rows; only Type changes.
use airledger_engine::{parse_view, CellValue, SheetsRepository};

const VIEW: &str = "name: cardio_fix
datasource: gsheets
table: \"4x4\"
dimensions:
  - { name: date, type: string, expr: Date }
  - { name: type, type: string, expr: Type }
";

fn main() {
    let sa = std::fs::read_to_string(
        std::env::var("HOME").unwrap() + "/.config/ledger/service-account.json",
    ).unwrap();
    let repo = SheetsRepository::new(
        "1hiDgkewR-z7JCJ1yNkKpCB7klIY0HDGupLscmcUSnhQ".into(), &sa,
    ).unwrap();
    let view = parse_view(VIEW).unwrap();
    let mut fixed = 0;
    for r in repo.list(&view, None).unwrap() {
        let is_target = r.get("date").map(|v| v.to_display_string()) == Some("2026-08-27".into())
            && r.get("type").map(|v| v.to_display_string()) == Some("treadmill".into());
        if is_target {
            let mut e = r.clone();
            e.insert("type".into(), CellValue::String("outdoor running".into()));
            repo.update(&view, e).unwrap();
            fixed += 1;
        }
    }
    println!("flipped {fixed} rows to outdoor running");
}
