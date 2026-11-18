use serde_json::Value;

pub const MIN_PREVIEW_BYTES: usize = 256;

fn marker(omitted: usize) -> String {
    format!("…[Lens preview: {omitted} bytes omitted]")
}

fn floor_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    (0..=index)
        .rev()
        .find(|candidate| text.is_char_boundary(*candidate))
        .unwrap_or(0)
}

fn cut(text: &str, keep: usize) -> String {
    let end = floor_boundary(text, keep);
    if end == text.len() {
        return text.to_owned();
    }
    format!("{}{}", &text[..end], marker(text.len() - end))
}

fn longest_leaf(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(longest_leaf).max().unwrap_or(0),
        Value::Object(fields) => fields.values().map(longest_leaf).max().unwrap_or(0),
        _ => 0,
    }
}

fn capped(value: &Value, keep: usize) -> Value {
    match value {
        Value::String(text) => Value::String(cut(text, keep)),
        Value::Array(items) => Value::Array(items.iter().map(|item| capped(item, keep)).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), capped(item, keep)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn largest_fitting(
    budget: usize,
    upper: usize,
    render: impl Fn(usize) -> String,
) -> Option<String> {
    let fits = |keep: usize| Some(render(keep)).filter(|text| text.len() <= budget);
    let mut best = fits(0)?;
    let (mut low, mut high) = (0, upper);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        match fits(middle) {
            Some(text) => {
                best = text;
                low = middle;
            }
            None => high = middle - 1,
        }
    }
    Some(best)
}

fn structured(value: &Value, budget: usize) -> Option<String> {
    largest_fitting(budget, longest_leaf(value), |keep| {
        capped(value, keep).to_string()
    })
}

fn quoted(content: &str, budget: usize) -> String {
    largest_fitting(budget, content.len(), |keep| {
        Value::String(cut(content, keep)).to_string()
    })
    .unwrap_or_default()
}

fn plain(content: &str, budget: usize) -> String {
    largest_fitting(budget, content.len(), |keep| cut(content, keep)).unwrap_or_default()
}

/// Bounds `content` to `budget` UTF-8 bytes. JSON stays valid JSON: long string leaves are cut and
/// marked, or the whole document becomes one marked JSON string when its structure alone overflows.
pub fn preview(content: &str, budget: usize) -> String {
    let budget = budget.max(MIN_PREVIEW_BYTES);
    if content.len() <= budget {
        return content.to_owned();
    }
    match serde_json::from_str::<Value>(content) {
        Ok(value) => structured(&value, budget).unwrap_or_else(|| quoted(content, budget)),
        Err(_) => plain(content, budget),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::*;

    #[rstest]
    fn small_content_is_unchanged() {
        assert_eq!(
            preview("[{\"content\":\"hi\"}]", 1024),
            "[{\"content\":\"hi\"}]"
        );
    }

    #[rstest]
    #[case::ascii("a")]
    #[case::two_byte("é")]
    #[case::three_byte("雪")]
    #[case::four_byte("🦀")]
    fn plain_text_is_cut_on_a_char_boundary_within_budget(#[case] unit: &str) {
        let content = unit.repeat(1000);
        let result = preview(&content, 300);

        assert!(result.len() <= 300);
        let kept = result.split("…[Lens preview: ").next().unwrap();
        assert!(content.starts_with(kept));
        assert!(!kept.is_empty());
        assert!(result.ends_with(&format!("{} bytes omitted]", content.len() - kept.len())));
    }

    #[rstest]
    fn json_messages_keep_their_structure_and_mark_each_cut_leaf() {
        let long = "雪".repeat(2000);
        let content = json!([
            {"role": "system", "content": "short"},
            {"role": "user", "content": long},
        ])
        .to_string();

        let result = preview(&content, 1024);
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert!(result.len() <= 1024);
        assert_eq!(parsed[0], json!({"role": "system", "content": "short"}));
        assert_eq!(parsed[1]["role"], "user");
        let text = parsed[1]["content"].as_str().unwrap();
        let kept = text.split("…[Lens preview: ").next().unwrap();
        assert!(long.starts_with(kept) && !kept.is_empty());
        assert!(text.ends_with(&format!("{} bytes omitted]", long.len() - kept.len())));
    }

    #[rstest]
    fn json_whose_structure_overflows_becomes_one_marked_string() {
        let content = Value::Array((0..400).map(|index| json!({"i": index})).collect()).to_string();

        let result = preview(&content, 300);
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert!(result.len() <= 300);
        let text = parsed.as_str().unwrap();
        assert!(content.starts_with(text.split("…[Lens preview: ").next().unwrap()));
        assert!(text.contains("bytes omitted]"));
    }

    #[rstest]
    fn budget_below_minimum_is_raised_to_minimum() {
        let content = "x".repeat(10_000);
        let result = preview(&content, 1);
        assert!(result.len() <= MIN_PREVIEW_BYTES);
        assert!(result.len() > MIN_PREVIEW_BYTES / 2);
    }
}
