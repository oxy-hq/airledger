//! Read-only audit of the 4x4 cardio tab around Aug 27.
//! All dims typed string so we see raw formatted cell text.
use airledger_engine::{parse_view, CellValue, SheetsRepository};

const VIEW: &str = "name: cardio_audit
datasource: gsheets
table: \"4x4\"
dimensions:
  - { name: id, type: string, expr: id }
  - { name: date, type: string, expr: Date }
  - { name: start, type: string, expr: Start Time }
  - { name: type, type: string, expr: Type }
  - { name: total, type: string, expr: Total Time }
  - { name: speed, type: string, expr: Treadmill Speed }
  - { name: incline, type: string, expr: Treadmill Incline }
";

fn main() {
    let sa = std::fs::read_to_string(
        std::env::var("HOME").unwrap() + "/.config/ledger/service-account.json",
    ).unwrap();
    let repo = SheetsRepository::new(
        "1C1rSudguUv00gYsb7i82XV6OM1V2KSZ4BGwMliwKDG4".into(), &sa,
    ).unwrap();
    let view = parse_view(VIEW).unwrap();
    let rows = repo.list(&view, None).unwrap();
    println!("total data rows: {}", rows.len());
    let g = |r: &std::collections::BTreeMap<String, CellValue>, k: &str| {
        r.get(k).map(|v| v.to_display_string()).unwrap_or_default()
    };
    for r in rows.iter().take(8) {
        let d = g(r, "date");
        {
            println!(
                "__row={} date={:?} start={:?} type={:?} total={:?} speed={:?} incline={:?} id={:?}",
                g(r, "__row"), d, g(r, "start"), g(r, "type"), g(r, "total"),
                g(r, "speed"), g(r, "incline"),
                &g(r, "id").chars().take(8).collect::<String>(),
            );
        }
    }
}
