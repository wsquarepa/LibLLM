//! Row formatters for the `db sql` and `db shell` output paths.
//!
//! Four formats share a single trait so `--format <name>` and the in-shell
//! `.mode <name>` command pick from the same set.

use rusqlite::types::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Table,
    Pipe,
    Csv,
    Json,
}

impl Format {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "table" => Some(Self::Table),
            "pipe" => Some(Self::Pipe),
            "csv" => Some(Self::Csv),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    pub fn formatter(self) -> Box<dyn RowFormatter> {
        match self {
            Self::Table => Box::new(TableFormatter),
            Self::Pipe => Box::new(PipeFormatter),
            Self::Csv => Box::new(CsvFormatter),
            Self::Json => Box::new(JsonFormatter),
        }
    }
}

pub trait RowFormatter {
    fn format(&self, headers: &[String], rows: &[Vec<Value>], show_headers: bool) -> String;
}

fn cell_to_display(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_owned(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        Value::Text(s) => s.clone(),
        Value::Blob(b) => format!("<blob:{} bytes>", b.len()),
    }
}

pub struct TableFormatter;
impl RowFormatter for TableFormatter {
    fn format(&self, headers: &[String], rows: &[Vec<Value>], show_headers: bool) -> String {
        let column_count = headers.len();
        let mut widths = vec![0usize; column_count];
        if show_headers {
            for (idx, header) in headers.iter().enumerate() {
                widths[idx] = widths[idx].max(header.chars().count());
            }
        }
        let display: Vec<Vec<String>> = rows
            .iter()
            .map(|row| row.iter().map(cell_to_display).collect())
            .collect();
        for row in &display {
            for (idx, cell) in row.iter().enumerate() {
                widths[idx] = widths[idx].max(cell.chars().count());
            }
        }

        let mut out = String::new();
        if show_headers {
            for (idx, header) in headers.iter().enumerate() {
                if idx > 0 {
                    out.push_str("  ");
                }
                let pad = widths[idx] - header.chars().count();
                out.push_str(header);
                for _ in 0..pad {
                    out.push(' ');
                }
            }
            out.push('\n');
            for (idx, width) in widths.iter().enumerate() {
                if idx > 0 {
                    out.push_str("  ");
                }
                for _ in 0..*width {
                    out.push('-');
                }
            }
            out.push('\n');
        }
        for row in &display {
            for (idx, cell) in row.iter().enumerate() {
                if idx > 0 {
                    out.push_str("  ");
                }
                let pad = widths[idx] - cell.chars().count();
                out.push_str(cell);
                for _ in 0..pad {
                    out.push(' ');
                }
            }
            out.push('\n');
        }
        out
    }
}

pub struct PipeFormatter;
impl RowFormatter for PipeFormatter {
    fn format(&self, headers: &[String], rows: &[Vec<Value>], show_headers: bool) -> String {
        let mut out = String::new();
        if show_headers {
            out.push_str(&headers.join("|"));
            out.push('\n');
        }
        for row in rows {
            let cells: Vec<String> = row.iter().map(cell_to_display).collect();
            out.push_str(&cells.join("|"));
            out.push('\n');
        }
        out
    }
}

fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        let escaped = field.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        field.to_owned()
    }
}

pub struct CsvFormatter;
impl RowFormatter for CsvFormatter {
    fn format(&self, headers: &[String], rows: &[Vec<Value>], show_headers: bool) -> String {
        let mut out = String::new();
        if show_headers {
            let escaped: Vec<String> = headers.iter().map(|h| csv_escape(h)).collect();
            out.push_str(&escaped.join(","));
            out.push('\n');
        }
        for row in rows {
            let cells: Vec<String> = row
                .iter()
                .map(|value| csv_escape(&cell_to_display(value)))
                .collect();
            out.push_str(&cells.join(","));
            out.push('\n');
        }
        out
    }
}

fn json_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        Value::Text(s) => json_string(s),
        Value::Blob(b) => json_string(&format!("<blob:{} bytes>", b.len())),
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub struct JsonFormatter;
impl RowFormatter for JsonFormatter {
    fn format(&self, headers: &[String], rows: &[Vec<Value>], show_headers: bool) -> String {
        let mut out = String::from("[");
        for (row_idx, row) in rows.iter().enumerate() {
            if row_idx > 0 {
                out.push(',');
            }
            if show_headers {
                out.push('{');
                for (col_idx, value) in row.iter().enumerate() {
                    if col_idx > 0 {
                        out.push(',');
                    }
                    out.push_str(&json_string(&headers[col_idx]));
                    out.push(':');
                    out.push_str(&json_value(value));
                }
                out.push('}');
            } else {
                out.push('[');
                for (col_idx, value) in row.iter().enumerate() {
                    if col_idx > 0 {
                        out.push(',');
                    }
                    out.push_str(&json_value(value));
                }
                out.push(']');
            }
        }
        out.push_str("]\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::types::Value;

    fn fixture() -> (Vec<String>, Vec<Vec<Value>>) {
        let headers = vec!["id".to_owned(), "name".to_owned(), "note".to_owned()];
        let rows = vec![
            vec![
                Value::Integer(1),
                Value::Text("Alice".to_owned()),
                Value::Null,
            ],
            vec![
                Value::Integer(2),
                Value::Text("Bo,b".to_owned()),
                Value::Text("hi\n".to_owned()),
            ],
        ];
        (headers, rows)
    }

    #[test]
    fn table_with_headers() {
        let (headers, rows) = fixture();
        let out = TableFormatter.format(&headers, &rows, true);
        let expected = "id  name   note\n--  -----  ----\n1   Alice  NULL\n2   Bo,b   hi\n \n";
        assert_eq!(out, expected);
    }

    #[test]
    fn pipe_with_headers() {
        let (headers, rows) = fixture();
        let out = PipeFormatter.format(&headers, &rows, true);
        assert_eq!(out, "id|name|note\n1|Alice|NULL\n2|Bo,b|hi\n\n");
    }

    #[test]
    fn csv_quotes_when_needed() {
        let (headers, rows) = fixture();
        let out = CsvFormatter.format(&headers, &rows, true);
        assert_eq!(out, "id,name,note\n1,Alice,NULL\n2,\"Bo,b\",\"hi\n\"\n");
    }

    #[test]
    fn csv_doubles_embedded_quotes() {
        let headers = vec!["v".to_owned()];
        let rows = vec![vec![Value::Text("say \"hi\"".to_owned())]];
        let out = CsvFormatter.format(&headers, &rows, false);
        assert_eq!(out, "\"say \"\"hi\"\"\"\n");
    }

    #[test]
    fn pipe_handles_real_and_blob() {
        let headers = vec!["r".to_owned(), "b".to_owned()];
        let rows = vec![vec![Value::Real(2.5), Value::Blob(vec![0x00, 0x01, 0x02])]];
        let out = PipeFormatter.format(&headers, &rows, false);
        assert_eq!(out, "2.5|<blob:3 bytes>\n");
    }

    #[test]
    fn json_handles_real_and_blob() {
        let headers = vec!["r".to_owned(), "b".to_owned()];
        let rows = vec![vec![Value::Real(2.5), Value::Blob(vec![0x00, 0x01, 0x02])]];
        let out = JsonFormatter.format(&headers, &rows, true);
        assert_eq!(out, "[{\"r\":2.5,\"b\":\"<blob:3 bytes>\"}]\n");
    }

    #[test]
    fn json_objects_with_headers() {
        let (headers, rows) = fixture();
        let out = JsonFormatter.format(&headers, &rows, true);
        assert_eq!(
            out,
            "[{\"id\":1,\"name\":\"Alice\",\"note\":null},{\"id\":2,\"name\":\"Bo,b\",\"note\":\"hi\\n\"}]\n"
        );
    }

    #[test]
    fn json_arrays_without_headers() {
        let (headers, rows) = fixture();
        let out = JsonFormatter.format(&headers, &rows, false);
        assert_eq!(out, "[[1,\"Alice\",null],[2,\"Bo,b\",\"hi\\n\"]]\n");
    }

    #[test]
    fn table_without_headers() {
        let (headers, rows) = fixture();
        let out = TableFormatter.format(&headers, &rows, false);
        assert_eq!(out, "1  Alice  NULL\n2  Bo,b   hi\n \n");
    }

    #[test]
    fn pipe_without_headers() {
        let (headers, rows) = fixture();
        let out = PipeFormatter.format(&headers, &rows, false);
        assert_eq!(out, "1|Alice|NULL\n2|Bo,b|hi\n\n");
    }

    #[test]
    fn csv_without_headers() {
        let (headers, rows) = fixture();
        let out = CsvFormatter.format(&headers, &rows, false);
        assert_eq!(out, "1,Alice,NULL\n2,\"Bo,b\",\"hi\n\"\n");
    }

    #[test]
    fn parse_format() {
        assert_eq!(Format::parse("table"), Some(Format::Table));
        assert_eq!(Format::parse("pipe"), Some(Format::Pipe));
        assert_eq!(Format::parse("csv"), Some(Format::Csv));
        assert_eq!(Format::parse("json"), Some(Format::Json));
        assert_eq!(Format::parse("xml"), None);
    }
}
