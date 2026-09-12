use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use serde_json::Value;
use walkdir::WalkDir;

use crate::protocol::SessionSummary;

pub fn scan(agent_dir: &Path) -> Vec<SessionSummary> {
    let root = agent_dir.join("sessions");
    let mut sessions = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|entry| parse_session(entry.path()))
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
    sessions
}

fn parse_session(path: &Path) -> Option<SessionSummary> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok();
    let reader = BufReader::new(file);
    let mut header: Option<Value> = None;
    let mut title: Option<String> = None;
    let mut first_user: Option<String> = None;
    let mut message_count = 0usize;
    let mut last_timestamp: Option<String> = None;

    for line in reader.lines().map_while(Result::ok) {
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if header.is_none() {
            if value.get("type").and_then(Value::as_str) != Some("session") {
                return None;
            }
            header = Some(value);
            continue;
        }
        if value.get("type").and_then(Value::as_str) == Some("session_info") {
            title = value
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned);
        }
        if value.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        message_count += 1;
        if let Some(timestamp) = message_timestamp(&value) {
            last_timestamp = Some(timestamp);
        }
        let message = value.get("message")?;
        if first_user.is_none() && message.get("role").and_then(Value::as_str) == Some("user") {
            first_user = extract_text(message.get("content")?).map(|text| compact_title(&text));
        }
    }

    let header = header?;
    let catalog_id = header.get("id")?.as_str()?.to_owned();
    let cwd = header
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let created_at = header
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let modified_at =
        last_timestamp.unwrap_or_else(|| modified_time(metadata.and_then(|m| m.modified().ok())));
    Some(SessionSummary {
        catalog_id,
        runtime_id: None,
        title: title.or(first_user).unwrap_or_else(|| "新对话".to_owned()),
        cwd,
        created_at,
        modified_at,
        message_count,
        running: false,
        path: Some(path.to_path_buf()),
    })
}

fn message_timestamp(entry: &Value) -> Option<String> {
    if let Some(ms) = entry
        .get("message")
        .and_then(|message| message.get("timestamp"))
        .and_then(Value::as_i64)
    {
        return DateTime::<Utc>::from_timestamp_millis(ms).map(|time| time.to_rfc3339());
    }
    entry
        .get("timestamp")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn extract_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }
    let text = content
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

pub fn compact_title(input: &str) -> String {
    let one_line = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = one_line.chars().take(42).collect::<String>();
    if one_line.chars().count() > 42 {
        title.push('…');
    }
    if title.is_empty() {
        "新对话".to_owned()
    } else {
        title
    }
}

fn modified_time(time: Option<SystemTime>) -> String {
    time.map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}

pub fn find_by_id(agent_dir: &Path, catalog_id: &str) -> Option<SessionSummary> {
    scan(agent_dir)
        .into_iter()
        .find(|session| session.catalog_id == catalog_id)
}

#[cfg(test)]
mod tests {
    use super::compact_title;

    #[test]
    fn title_is_single_line_and_bounded() {
        let title = compact_title("  hello\nworld   from pi  ");
        assert_eq!(title, "hello world from pi");
    }
}
