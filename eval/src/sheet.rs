use std::{collections::BTreeSet, fs::File, io::Write, path::Path};

use serde_json::{Value, json};

use crate::{
    annotation::{self, HEADERS, SheetRow},
    output, stream,
};

pub fn export(messages: &Path, out: &Path, exit_code: Option<i32>) -> Result<Value, String> {
    let messages = stream::messages(messages, exit_code)?;
    let mut ids = BTreeSet::new();
    for message in &messages {
        annotation::message_id(message.message_id.as_ref())?;
        if !ids.insert(&message.message_id) {
            return Err("duplicate message_id in messages stream".into());
        }
    }
    let skipped = messages.iter().filter(|message| message.recalled).count();
    let count = messages.len() - skipped;
    output::file(out, |file| {
        // Excel detects UTF-8 from the BOM; csv handles embedded CR/LF and quotes.
        file.write_all(b"\xef\xbb\xbf")
            .map_err(|error| format!("cannot write CSV: {error}"))?;
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(file);
        writer
            .write_record(HEADERS)
            .map_err(|error| format!("cannot write CSV header: {error}"))?;
        for message in messages.into_iter().filter(|message| !message.recalled) {
            writer
                .serialize(SheetRow::unlabelled(message))
                .map_err(|error| format!("cannot write CSV row: {error}"))?;
        }
        writer
            .flush()
            .map_err(|error| format!("cannot flush CSV: {error}"))
    })?;
    Ok(
        json!({"command":"export-sheet","valid":true,"out":out,"messages":count,"recalled_skipped":skipped,
        "exit_code_source":if exit_code.is_some() {"argument"} else {"stream_only"}}),
    )
}

pub fn import(path: &Path, out: &Path) -> Result<Value, String> {
    let file = File::open(path).map_err(|error| format!("cannot open annotation CSV: {error}"))?;
    let mut reader = csv::ReaderBuilder::new().from_reader(file);
    let headers = reader.headers().map_err(|_| "invalid CSV header")?;
    let header_set: BTreeSet<_> = headers.iter().collect();
    if headers.len() != HEADERS.len() || header_set != HEADERS.into_iter().collect() {
        return Err("CSV must contain exactly the export-sheet columns, once each".into());
    }
    let mut messages = Vec::new();
    let mut items = Vec::new();
    let mut message_ids = BTreeSet::new();
    let mut item_ids = BTreeSet::new();
    for (index, record) in reader.deserialize::<SheetRow>().enumerate() {
        let number = index + 2;
        let row = record.map_err(|_| {
            format!("CSV record {number}: invalid UTF-8, structure or column count")
        })?;
        let (message, row_items) = row
            .labelled()
            .map_err(|error| format!("CSV record {number}: {error}"))?;
        if !message_ids.insert(message.message_id.clone()) {
            return Err(format!("CSV record {number}: duplicate message_id"));
        }
        for item in &row_items {
            if !item_ids.insert(item.item_id.clone()) {
                return Err(format!(
                    "CSV record {number}: duplicate item_id; define each item exactly once"
                ));
            }
            if !item.anchors.contains(&message.message_id) {
                return Err(format!(
                    "CSV record {number}: the defining row must be one of the item's anchors"
                ));
            }
        }
        messages.push(message);
        items.extend(row_items);
    }
    for item in &items {
        if item
            .anchors
            .iter()
            .any(|anchor| !message_ids.contains(anchor))
        {
            return Err(
                "item anchor refers to a message absent from this sheet (possibly recalled)".into(),
            );
        }
    }
    output::gold(out, &messages, &items)?;
    Ok(
        json!({"command":"import-sheet","valid":true,"out":out,"messages":messages.len(),"items":items.len()}),
    )
}
