//! Benchmark: full-view hydration through sync_views (33k rows).
use std::cell::RefCell;
use std::time::Instant;

use airledger_engine::sheets::SheetsError;
use airledger_engine::store::Store;
use airledger_engine::sync::{sync_views, SyncRemote};
use airledger_engine::value::{CellValue, Record};
use airledger_engine::{parse_view, ViewSchema};

struct Fake(RefCell<Vec<Record>>);
impl SyncRemote for Fake {
    fn ensure(&self, _v: &ViewSchema) -> Result<(), SheetsError> { Ok(()) }
    fn pull(&self, _v: &ViewSchema) -> Result<Vec<Record>, SheetsError> {
        Ok(self.0.borrow().iter().enumerate().map(|(i, r)| {
            let mut r = r.clone();
            r.insert("__row".into(), CellValue::Int(i as i64));
            r
        }).collect())
    }
    fn push_update(&self, _v: &ViewSchema, _r: &Record) -> Result<(), SheetsError> { Ok(()) }
    fn push_insert(&self, _v: &ViewSchema, _r: &Record) -> Result<(), SheetsError> { Ok(()) }
    fn push_delete(&self, _v: &ViewSchema, _i: usize) -> Result<(), SheetsError> { Ok(()) }
}

fn main() {
    let dir = std::env::temp_dir().join("airledger-hydrate-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("h.db");
    std::fs::remove_file(&path).ok();
    let store = Store::open(path.to_str().unwrap()).unwrap();
    let view = parse_view("name: strength\ndatasource: gsheets\ntable: strength\ndimensions:\n  - { name: id, type: string, expr: id }\n  - { name: exercise, type: string, expr: Exercise }\n  - { name: weight, type: number, expr: Weight }\n").unwrap();
    let rows: Vec<Record> = (0..33_000).map(|i| {
        let mut r = Record::new();
        r.insert("id".into(), CellValue::String(format!("id-{i}")));
        r.insert("exercise".into(), CellValue::String(format!("Exercise {}", i % 40)));
        r.insert("weight".into(), CellValue::Float(135.0));
        r
    }).collect();
    let remote = Fake(RefCell::new(rows));

    let t = Instant::now();
    let res = sync_views(&store, &remote, std::slice::from_ref(&view));
    println!("hydrate 33k rows via sync: {:?} (pulled {})", t.elapsed(), res[0].pulled);
    let t = Instant::now();
    let res = sync_views(&store, &remote, std::slice::from_ref(&view));
    println!("steady-state sync (no changes): {:?} (pulled {})", t.elapsed(), res[0].pulled);
}
