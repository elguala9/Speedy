use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::{
    db,
    tokenize::SearchType,
};

#[derive(Serialize)]
struct ResultEntry {
    file: String,
    line: u32,
    col_start: u32,
    col_end: u32,
}

#[derive(Serialize)]
struct QueryResult {
    symbol: String,
    #[serde(rename = "type")]
    search_type: String,
    ext: Option<String>,
    count: usize,
    results: Vec<ResultEntry>,
}

pub fn run_query(
    conn: &Connection,
    symbol: &str,
    search_type: &SearchType,
    ext_filter: Option<&str>,
    ignore_case: bool,
) -> Result<()> {
    let occurrences = db::query_occurrences(conn, symbol, search_type, ext_filter, ignore_case)?;

    let entries: Vec<ResultEntry> = occurrences
        .into_iter()
        .map(|o| ResultEntry {
            file: o.file,
            line: o.line,
            col_start: o.col_start,
            col_end: o.col_end,
        })
        .collect();

    let result = QueryResult {
        symbol: symbol.to_string(),
        search_type: search_type.as_str().to_string(),
        ext: ext_filter.map(|e| e.to_string()),
        count: entries.len(),
        results: entries,
    };

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
