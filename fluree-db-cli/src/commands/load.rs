//! `fluree load` — stream a local CSV into a ledger as batched per-row Cypher
//! upserts (the `LOAD CSV` analog).
//!
//! The CLI is the component that actually holds the file, so it reads the CSV
//! locally, batches the rows, and sends each batch to the ledger — local or
//! remote — as one `UNWIND $batch AS row <template>` transaction (one commit
//! per batch). This sidesteps server-side file access entirely: the server
//! only ever receives ordinary parameterized Cypher writes. Rows are maps
//! keyed by CSV column; every value is a string (cast in the template with
//! `toInteger()` / `toFloat()`), and an empty cell is `null`.

use crate::context::{self, LedgerMode};
use crate::error::{CliError, CliResult};
use fluree_db_api::server_defaults::FlureeDir;
use serde_json::{json, Map, Value};
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    ledger_flag: Option<&str>,
    from: &Path,
    template: &str,
    batch_size: usize,
    field_terminator: &str,
    dirs: &FlureeDir,
    remote_flag: Option<&str>,
    direct: bool,
) -> CliResult<()> {
    if batch_size == 0 {
        return Err(CliError::Usage("--batch-size must be at least 1".into()));
    }
    let delimiter = single_byte_delimiter(field_terminator)?;

    // Wrap the per-row template. The batch rides in as the `$batch` parameter;
    // `UNWIND … AS row` binds one row map at a time, exactly like `LOAD CSV …
    // AS row`.
    let wrapped = format!("UNWIND $batch AS row\n{template}");

    // Resolve where the write lands: explicit --remote, or the local ledger
    // (auto-routed to a running local server unless --direct), mirroring
    // `fluree update`.
    let mode = if let Some(remote_name) = remote_flag {
        let alias = context::resolve_ledger(ledger_flag, dirs)?;
        context::build_remote_mode(remote_name, &alias, dirs).await?
    } else {
        let mode = context::resolve_ledger_mode(ledger_flag, dirs).await?;
        if direct {
            mode
        } else {
            context::try_server_route(mode, dirs)
        }
    };

    match mode {
        LedgerMode::Tracked {
            client,
            remote_alias,
            remote_name,
            ..
        } => {
            let mut reader = csv_reader(from, delimiter)?;
            let headers = header_row(&mut reader)?;
            let (mut total, mut commits) = (0usize, 0usize);
            let mut batch: Vec<Value> = Vec::with_capacity(batch_size);
            for record in reader.records() {
                batch.push(record_to_row(&headers, &record.map_err(csv_err)?));
                if batch.len() >= batch_size {
                    let params = json!({ "batch": std::mem::take(&mut batch) });
                    client
                        .update_cypher(&remote_alias, &wrapped, params.as_object())
                        .await?;
                    commits += 1;
                    total += batch_len(&params);
                    report_progress(total);
                    batch = Vec::with_capacity(batch_size);
                }
            }
            if !batch.is_empty() {
                let params = json!({ "batch": batch });
                client
                    .update_cypher(&remote_alias, &wrapped, params.as_object())
                    .await?;
                commits += 1;
                total += batch_len(&params);
            }
            context::persist_refreshed_tokens(&client, &remote_name, dirs).await;
            println!("Loaded {total} rows into `{remote_alias}` in {commits} commit(s)");
        }
        LedgerMode::Local { fluree, alias } => {
            let mut ledger = fluree.ledger(&alias).await?;
            let mut reader = csv_reader(from, delimiter)?;
            let headers = header_row(&mut reader)?;
            let (mut total, mut commits) = (0usize, 0usize);
            let mut batch: Vec<Value> = Vec::with_capacity(batch_size);
            for record in reader.records() {
                batch.push(record_to_row(&headers, &record.map_err(csv_err)?));
                if batch.len() >= batch_size {
                    let params = json!({ "batch": std::mem::take(&mut batch) });
                    let result = fluree
                        .transact_cypher_with_params(ledger, &wrapped, params.as_object())
                        .await?;
                    ledger = result.ledger;
                    commits += 1;
                    total += batch_len(&params);
                    report_progress(total);
                    batch = Vec::with_capacity(batch_size);
                }
            }
            if !batch.is_empty() {
                let params = json!({ "batch": batch });
                let result = fluree
                    .transact_cypher_with_params(ledger, &wrapped, params.as_object())
                    .await?;
                commits += 1;
                total += batch_len(&params);
                let _ = result;
            }
            println!("Loaded {total} rows into `{alias}` in {commits} commit(s)");
        }
    }

    Ok(())
}

/// One CSV field delimiter byte. Cypher/CSV field terminators are single
/// characters; multi-byte input is a user error.
fn single_byte_delimiter(s: &str) -> CliResult<u8> {
    let bytes = s.as_bytes();
    if bytes.len() != 1 {
        return Err(CliError::Usage(format!(
            "--field-terminator must be a single character, got {s:?}"
        )));
    }
    Ok(bytes[0])
}

fn csv_reader(path: &Path, delimiter: u8) -> CliResult<csv::Reader<std::fs::File>> {
    csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(true)
        .flexible(true)
        .from_path(path)
        .map_err(csv_err)
}

fn header_row(reader: &mut csv::Reader<std::fs::File>) -> CliResult<Vec<String>> {
    let headers = reader.headers().map_err(csv_err)?;
    if headers.is_empty() {
        return Err(CliError::Usage(
            "CSV has no header row — the first line must name the columns".into(),
        ));
    }
    Ok(headers.iter().map(str::to_string).collect())
}

/// One CSV record → a `row` map: each column keyed to its cell as a string
/// (empty → `null`, matching Neo4j `LOAD CSV`).
fn record_to_row(headers: &[String], record: &csv::StringRecord) -> Value {
    let mut obj = Map::with_capacity(headers.len());
    for (header, cell) in headers.iter().zip(record.iter()) {
        let value = if cell.is_empty() {
            Value::Null
        } else {
            Value::String(cell.to_string())
        };
        obj.insert(header.clone(), value);
    }
    Value::Object(obj)
}

fn batch_len(params: &Value) -> usize {
    params
        .get("batch")
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

fn report_progress(total: usize) {
    eprintln!("  … {total} rows");
}

fn csv_err(e: csv::Error) -> CliError {
    CliError::Usage(format!("CSV read error: {e}"))
}
