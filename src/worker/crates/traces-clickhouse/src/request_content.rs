const PAGE_CHARS: usize = 8000;
const OMITTED: &str = "\n[... content omitted ...]\n";

fn chars(text: &str, skip: usize, take: usize) -> String {
    text.chars().skip(skip).take(take).collect()
}

fn excerpt(value: &str, budget: usize) -> String {
    let length = value.chars().count();
    if length <= budget {
        return value.to_owned();
    }
    let head = budget / 3;
    let tail = budget - head;
    format!(
        "{}{OMITTED}{}",
        chars(value, 0, head),
        chars(value, length - tail, tail)
    )
}

pub fn request_text(input: &str, output: &str, error: &str) -> String {
    format!("Input: {input}\nOutput: {output}\nError: {error}")
}

/// Same paging as the requests half of `lens_content.sql`, applied to resolved bodies: offset 0 is
/// the whole record, offset 1 an excerpt when it exceeds a page, and offset n page `n - 1` on.
pub fn request_page(input: &str, output: &str, error: &str, offset: u32) -> (String, bool) {
    let full = request_text(input, output, error);
    let length = full.chars().count();
    let start = (offset as usize).saturating_sub(1).max(1);
    let content = match offset {
        0 => full.clone(),
        1 if length > PAGE_CHARS => request_text(
            &excerpt(input, 2000),
            &excerpt(output, 5000),
            &excerpt(error, 500),
        ),
        _ => chars(&full, start - 1, PAGE_CHARS),
    };
    (content, offset != 0 && length >= start + PAGE_CHARS)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn whole_record_at_offset_zero() {
        assert_eq!(
            request_page("in", "out", "", 0),
            ("Input: in\nOutput: out\nError: ".to_owned(), false)
        );
    }

    #[rstest]
    fn short_record_is_one_untruncated_page() {
        assert_eq!(
            request_page("in", "out", "", 1),
            ("Input: in\nOutput: out\nError: ".to_owned(), false)
        );
    }

    #[rstest]
    fn long_record_excerpt_keeps_head_and_tail() {
        let input = format!("{}{}", "a".repeat(5000), "雪".repeat(5000));
        let (content, truncated) = request_page(&input, "", "", 1);

        assert!(truncated);
        assert!(content.starts_with(&format!("Input: {}", "a".repeat(666))));
        assert!(content.contains(&format!("{OMITTED}{}", "雪".repeat(1334))));
    }

    #[rstest]
    #[case::second_page(8001, "雪", false)]
    #[case::first_page(2, "Input: a", true)]
    fn later_offsets_page_by_characters(
        #[case] offset: u32,
        #[case] first: &str,
        #[case] truncated: bool,
    ) {
        let input = format!("{}{}", "a".repeat(7992), "雪".repeat(7000));
        let (content, more) = request_page(&input, "", "", offset);

        assert!(content.starts_with(first));
        assert_eq!(more, truncated);
        assert!(content.chars().count() <= 8000);
    }
}
