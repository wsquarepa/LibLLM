//! User-defined macro expansion with positional argument substitution.

const MAX_MACRO_PLACEHOLDER_RANGE: usize = 256;

#[derive(Debug, PartialEq)]
pub(super) enum Placeholder {
    All,
    Single(usize),
    Range(usize, usize),
    Greedy(usize),
}

pub(super) fn parse_placeholder(content: &str) -> Result<Placeholder, String> {
    let content = content.trim();
    if content.is_empty() {
        return Ok(Placeholder::All);
    }

    if let Some(rest) = content.strip_suffix("...") {
        if rest.is_empty() {
            return Err("Invalid placeholder: {{...}}".to_owned());
        }
        let n: usize = rest
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        if n == 0 {
            return Err("Placeholder indices start at 1".to_owned());
        }
        return Ok(Placeholder::Greedy(n));
    }

    if let Some(rest) = content.strip_suffix("..") {
        if rest.is_empty() {
            return Err("Invalid placeholder: {{..}}".to_owned());
        }
        let n: usize = rest
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        if n == 0 {
            return Err("Placeholder indices start at 1".to_owned());
        }
        return Ok(Placeholder::Greedy(n));
    }

    if let Some(dot_pos) = content.find("...") {
        let left = &content[..dot_pos];
        let right = &content[dot_pos + 3..];
        let a: usize = left
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        let b: usize = right
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        if a == 0 || b == 0 {
            return Err("Placeholder indices start at 1".to_owned());
        }
        if a > b {
            return Err(format!("Invalid range: {a}...{b} (start > end)"));
        }
        if b - a + 1 > MAX_MACRO_PLACEHOLDER_RANGE {
            return Err(format!(
                "Range {a}...{b} spans {} indices, exceeding the limit of {MAX_MACRO_PLACEHOLDER_RANGE}",
                b - a + 1
            ));
        }
        return Ok(Placeholder::Range(a, b));
    }

    if let Some(dot_pos) = content.find("..") {
        let left = &content[..dot_pos];
        let right = &content[dot_pos + 2..];
        let a: usize = left
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        let b: usize = right
            .parse()
            .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
        if a == 0 || b == 0 {
            return Err("Placeholder indices start at 1".to_owned());
        }
        if a > b {
            return Err(format!("Invalid range: {a}..{b} (start > end)"));
        }
        if b - a + 1 > MAX_MACRO_PLACEHOLDER_RANGE {
            return Err(format!(
                "Range {a}..{b} spans {} indices, exceeding the limit of {MAX_MACRO_PLACEHOLDER_RANGE}",
                b - a + 1
            ));
        }
        return Ok(Placeholder::Range(a, b));
    }

    let n: usize = content
        .parse()
        .map_err(|_| format!("Invalid placeholder: {{{{{content}}}}}"))?;
    if n == 0 {
        return Err("Placeholder indices start at 1".to_owned());
    }
    Ok(Placeholder::Single(n))
}

pub(super) enum ScanItem {
    Escaped(usize, usize),
    Placeholder(usize, usize, Placeholder),
}

pub(super) fn scan_template(template: &str) -> Result<Vec<ScanItem>, String> {
    let mut result = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 2 < bytes.len() && bytes[i + 1] == b'{' && bytes[i + 2] == b'{' {
            result.push(ScanItem::Escaped(i, i + 1));
            i += 1;
            continue;
        }
        if bytes[i] == b'{' && i > 0 && bytes[i - 1] == b'\\' {
            i += 1;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i;
            let inner_start = i + 2;
            let mut j = inner_start;
            let mut found = false;
            while j + 1 < bytes.len() {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    let content = &template[inner_start..j];
                    let placeholder = parse_placeholder(content)?;
                    result.push(ScanItem::Placeholder(start, j + 2, placeholder));
                    i = j + 2;
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    Ok(result)
}

pub(super) fn validate_placeholders(items: &[ScanItem]) -> Result<(), String> {
    let mut covered_ranges: Vec<(usize, usize)> = Vec::new();
    let mut singles: Vec<usize> = Vec::new();
    let mut has_all = false;

    for item in items {
        if let ScanItem::Placeholder(_, _, ph) = item {
            match ph {
                Placeholder::All => has_all = true,
                Placeholder::Single(n) => singles.push(*n),
                Placeholder::Range(a, b) => covered_ranges.push((*a, *b)),
                Placeholder::Greedy(a) => covered_ranges.push((*a, usize::MAX)),
            }
        }
    }

    if has_all {
        return Ok(());
    }

    for &n in &singles {
        for &(start, end) in &covered_ranges {
            if n >= start && n <= end {
                return Err(format!(
                    "Placeholder {{{{{n}}}}} overlaps with range {{{{{start}..{end}}}}}"
                ));
            }
        }
    }

    if singles.is_empty() && covered_ranges.is_empty() {
        return Ok(());
    }

    let mut max_idx: usize = 0;
    for &n in &singles {
        max_idx = max_idx.max(n);
    }
    for &(start, end) in &covered_ranges {
        max_idx = max_idx.max(start);
        if end != usize::MAX {
            max_idx = max_idx.max(end);
        }
    }

    if max_idx > MAX_MACRO_PLACEHOLDER_RANGE {
        return Err(format!(
            "Highest placeholder index {max_idx} exceeds the limit of {MAX_MACRO_PLACEHOLDER_RANGE}"
        ));
    }

    for idx in 1..=max_idx {
        let in_single = singles.contains(&idx);
        let in_range = covered_ranges.iter().any(|&(s, e)| idx >= s && idx <= e);
        if !in_single && !in_range {
            return Err(format!(
                "Gap at index {idx}: all indices from 1 to {max_idx} must be covered"
            ));
        }
    }

    Ok(())
}

pub fn expand_macro(template: &str, raw_args: &str) -> Result<String, String> {
    let items = scan_template(template)?;
    validate_placeholders(&items)?;

    let args: Vec<&str> = if raw_args.trim().is_empty() {
        Vec::new()
    } else {
        raw_args.split_whitespace().collect()
    };

    let mut result = String::with_capacity(template.len());
    let mut last_end = 0;

    for item in &items {
        match item {
            ScanItem::Escaped(start, skip_to) => {
                result.push_str(&template[last_end..*start]);
                last_end = *skip_to;
            }
            ScanItem::Placeholder(start, end, ph) => {
                result.push_str(&template[last_end..*start]);
                match ph {
                    Placeholder::All => result.push_str(raw_args),
                    Placeholder::Single(n) => {
                        if let Some(arg) = args.get(*n - 1) {
                            result.push_str(arg);
                        }
                    }
                    Placeholder::Range(a, b) => {
                        let from = (*a - 1).min(args.len());
                        let to = (*b).min(args.len());
                        let slice = &args[from..to];
                        result.push_str(&slice.join(" "));
                    }
                    Placeholder::Greedy(a) => {
                        let from = (*a - 1).min(args.len());
                        let slice = &args[from..];
                        result.push_str(&slice.join(" "));
                    }
                }
                last_end = *end;
            }
        }
    }

    result.push_str(&template[last_end..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_all_args() {
        let result = expand_macro("Refactor: {{}}", "fn foo() {}").unwrap();
        assert_eq!(result, "Refactor: fn foo() {}");
    }

    #[test]
    fn expand_single_positional() {
        let result = expand_macro("Hello {{1}}", "world").unwrap();
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn expand_multiple_positional() {
        let result = expand_macro("Compare {{1}} with {{2}}", "apples oranges").unwrap();
        assert_eq!(result, "Compare apples with oranges");
    }

    #[test]
    fn expand_positional_out_of_bounds() {
        let result = expand_macro("A={{1}} B={{2}} C={{3}}", "only two").unwrap();
        assert_eq!(result, "A=only B=two C=");
    }

    #[test]
    fn expand_range_two_dots() {
        let result = expand_macro("Items: {{1..3}}", "a b c d").unwrap();
        assert_eq!(result, "Items: a b c");
    }

    #[test]
    fn expand_range_three_dots() {
        let result = expand_macro("Items: {{1...3}}", "a b c d").unwrap();
        assert_eq!(result, "Items: a b c");
    }

    #[test]
    fn expand_range_out_of_bounds() {
        let result = expand_macro("Items: {{1..5}}", "a b").unwrap();
        assert_eq!(result, "Items: a b");
    }

    #[test]
    fn expand_greedy_two_dots() {
        let result = expand_macro("{{1}} {{2}} rest: {{3..}}", "a b c d e").unwrap();
        assert_eq!(result, "a b rest: c d e");
    }

    #[test]
    fn expand_greedy_three_dots() {
        let result = expand_macro("{{1}} {{2}} rest: {{3...}}", "a b c d e").unwrap();
        assert_eq!(result, "a b rest: c d e");
    }

    #[test]
    fn expand_greedy_out_of_bounds() {
        let result = expand_macro("{{1}} {{2}} rest: {{3..}}", "a b").unwrap();
        assert_eq!(result, "a b rest: ");
    }

    #[test]
    fn expand_mixed_positional_and_greedy() {
        let result = expand_macro("From {{1}} to {{2}}: {{3..}}", "en fr hello world").unwrap();
        assert_eq!(result, "From en to fr: hello world");
    }

    #[test]
    fn expand_no_placeholders() {
        let result = expand_macro("Just a template", "ignored args").unwrap();
        assert_eq!(result, "Just a template");
    }

    #[test]
    fn expand_empty_args() {
        let result = expand_macro("Hello {{}}", "").unwrap();
        assert_eq!(result, "Hello ");
    }

    #[test]
    fn expand_empty_args_positional() {
        let result = expand_macro("A={{1}}", "").unwrap();
        assert_eq!(result, "A=");
    }

    #[test]
    fn overlap_range_and_single_errors() {
        let result = expand_macro("{{1..3}} {{2}}", "a b c");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("overlaps"));
    }

    #[test]
    fn overlap_greedy_and_single_errors() {
        let result = expand_macro("{{3..}} {{5}}", "a b c d e");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("overlaps"));
    }

    #[test]
    fn single_outside_range_ok() {
        let result = expand_macro("{{1..2}} and {{3}}", "a b c").unwrap();
        assert_eq!(result, "a b and c");
    }

    #[test]
    fn zero_index_errors() {
        let result = expand_macro("{{0}}", "a");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("start at 1"));
    }

    #[test]
    fn invalid_placeholder_errors() {
        let result = expand_macro("{{abc}}", "a");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid placeholder"));
    }

    #[test]
    fn range_start_greater_than_end_errors() {
        let result = expand_macro("{{3..1}}", "a b c");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("start > end"));
    }

    #[test]
    fn all_placeholder_preserves_whitespace() {
        let result = expand_macro("Say: {{}}", "  hello   world  ").unwrap();
        assert_eq!(result, "Say:   hello   world  ");
    }

    #[test]
    fn multiple_all_placeholders() {
        let result = expand_macro("{{}} and {{}}", "test").unwrap();
        assert_eq!(result, "test and test");
    }

    #[test]
    fn gap_in_indices_errors() {
        let result = expand_macro("{{1}} {{3}}", "a b c");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Gap at index 2"));
    }

    #[test]
    fn range_exceeding_cap_errors() {
        let result = expand_macro("{{1..1000000000}}", "a");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeding the limit"));
    }

    #[test]
    fn range_at_cap_boundary_ok() {
        let result = expand_macro("{{1..256}}", "a b c");
        assert!(result.is_ok());
    }

    #[test]
    fn single_index_exceeding_cap_errors() {
        let result = expand_macro("{{257}}", "a");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds the limit"));
    }

    #[test]
    fn greedy_without_preceding_errors() {
        let result = expand_macro("{{5..}}", "a b c d e");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Gap at index 1"));
    }

    #[test]
    fn greedy_with_all_preceding_ok() {
        let result = expand_macro("{{1}} {{2}} {{3..}}", "a b c d e").unwrap();
        assert_eq!(result, "a b c d e");
    }

    #[test]
    fn escape_backslash_opening_braces() {
        let result = expand_macro("literal \\{{}} here", "args").unwrap();
        assert_eq!(result, "literal {{}} here");
    }

    #[test]
    fn escape_mixed_with_real_placeholder() {
        let result = expand_macro("real={{}} escaped=\\{{}}", "hello").unwrap();
        assert_eq!(result, "real=hello escaped={{}}");
    }

    #[test]
    fn range_covers_all_indices() {
        let result = expand_macro("{{1..3}} then {{4}}", "a b c d").unwrap();
        assert_eq!(result, "a b c then d");
    }

    #[test]
    fn greedy_covers_rest() {
        let result = expand_macro("first={{1}} rest={{2..}}", "a b c d").unwrap();
        assert_eq!(result, "first=a rest=b c d");
    }
}
