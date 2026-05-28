use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::{
    db,
    tokenize::SearchType,
};

#[derive(Serialize)]
pub struct ResultEntry {
    pub file: String,
    pub line: u32,
    pub col_start: u32,
    pub col_end: u32,
}

#[derive(Serialize)]
pub struct QueryResult {
    pub symbol: String,
    #[serde(rename = "type")]
    pub search_type: String,
    pub ext: Option<String>,
    pub count: usize,
    pub results: Vec<ResultEntry>,
}

/// Run a query and return the structured result (for the MCP / library callers).
pub fn query_json(
    conn: &Connection,
    symbol: &str,
    search_type: &SearchType,
    ext_filter: Option<&str>,
    ignore_case: bool,
) -> Result<QueryResult> {
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

    Ok(QueryResult {
        symbol: symbol.to_string(),
        search_type: search_type.as_str().to_string(),
        ext: ext_filter.map(|e| e.to_string()),
        count: entries.len(),
        results: entries,
    })
}

/// Run a query and print the result as pretty JSON (CLI path).
pub fn run_query(
    conn: &Connection,
    symbol: &str,
    search_type: &SearchType,
    ext_filter: Option<&str>,
    ignore_case: bool,
) -> Result<()> {
    let result = query_json(conn, symbol, search_type, ext_filter, ignore_case)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
