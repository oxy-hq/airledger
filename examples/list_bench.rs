//! Benchmark: Store::list on a strength-sized table (33k rows).
use airledger_engine::store::Store;
use airledger_engine::value::CellValue;
use airledger_engine::{parse_input_overlay, parse_view, apply_overlay};
use std::time::Instant;

fn main() {
    let dir = std::env::temp_dir().join("airledger-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bench.db");
    std::fs::remove_file(&path).ok();
    let store = Store::open(path.to_str().unwrap()).unwrap();
    let base = parse_view("name: strength\ndatasource: gsheets\ntable: strength\ndimensions:\n  - { name: id, type: string, expr: id }\n  - { name: date, type: date, expr: Date }\n  - { name: exercise, type: string, expr: Exercise }\n  - { name: weight, type: number, expr: Weight }\n  - { name: reps, type: number, expr: Reps }\n  - { name: start_time, type: string, expr: Start Time }\n  - { name: notes, type: string, expr: Notes }\n").unwrap();
    let overlay = parse_input_overlay("target: strength.view.yml\ndate_field: date\n").unwrap();
    let view = apply_overlay(base, overlay).unwrap();

    let t = Instant::now();
    for i in 0..33_000u32 {
        let day = (i % 1500) as i64;
        let date = chrono::NaiveDate::from_ymd_opt(2022, 1, 1).unwrap()
            + chrono::Duration::days(day);
        let mut r = std::collections::BTreeMap::new();
        r.insert("date".to_string(), CellValue::Date(date));
        r.insert("exercise".to_string(), CellValue::String(format!("Exercise {}", i % 40)));
        r.insert("weight".to_string(), CellValue::Float(135.0));
        r.insert("reps".to_string(), CellValue::Int(8));
        r.insert("start_time".to_string(), CellValue::String("9:00:00 AM".into()));
        r.insert("notes".to_string(), CellValue::String("some note text here".into()));
        let rec = store.create(&view, r).unwrap();
        let id = rec.get("id").unwrap().to_display_string();
        store.mark_synced(&view.name, &id, &rec, Some(i as i64)).unwrap();
    }
    println!("seed 33k rows: {:?}", t.elapsed());

    let on = chrono::NaiveDate::from_ymd_opt(2024, 5, 1).unwrap();
    for pass in 0..3 {
        let t = Instant::now();
        let rows = store.list(&view, Some(on)).unwrap();
        println!("list(on_date) pass {pass}: {:?} -> {} rows", t.elapsed(), rows.len());
    }
    let t = Instant::now();
    let rows = store.list(&view, None).unwrap();
    println!("list(all): {:?} -> {} rows", t.elapsed(), rows.len());
}
