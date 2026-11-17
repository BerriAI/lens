pub(crate) const OMISSION: &str = "\n[... content omitted ...]\n";
const MINIMUM_CHARACTERS: usize = 8;
const MINIMUM_FRAGMENT: usize = 12;
const MAXIMUM_PASSAGES: usize = 8;
const ANCHOR_CHARACTERS: [usize; 3] = [48, 24, 12];
const EXCERPT_CHARACTERS: usize = 240;
const ELISIONS: [&str; 8] = [
    "[...]",
    "(...)",
    "[\u{2026}]",
    "(\u{2026})",
    "...",
    "\u{2026}",
    "\\n",
    "\n",
];

struct Folded {
    text: String,
    starts: Vec<u32>,
    ends: Vec<u32>,
}

impl Folded {
    fn push(&mut self, character: char, start: usize, end: usize) {
        let character = match character {
            '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{2032}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{2033}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            other => other,
        };
        if character == '\u{2026}' {
            for _ in 0..3 {
                self.push('.', start, end);
            }
            return;
        }
        if character.is_whitespace() {
            if self.text.ends_with(' ') {
                if let Some(last) = self.ends.last_mut() {
                    *last = end as u32;
                }
                return;
            }
            self.text.push(' ');
            self.starts.push(start as u32);
            self.ends.push(end as u32);
            return;
        }
        if matches!(character, '*' | '`') {
            return;
        }
        let before = self.text.len();
        self.text.push(character);
        for _ in before..self.text.len() {
            self.starts.push(start as u32);
            self.ends.push(end as u32);
        }
    }

    fn slice<'a>(&self, source: &'a str, start: usize, end: usize) -> &'a str {
        &source[self.starts[start] as usize..self.ends[end - 1] as usize]
    }
}

fn fold(text: &str) -> Folded {
    let mut folded = Folded {
        text: String::with_capacity(text.len()),
        starts: Vec::with_capacity(text.len()),
        ends: Vec::with_capacity(text.len()),
    };
    let mut characters = text.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        if character != '\\' {
            folded.push(character, start, start + character.len_utf8());
            continue;
        }
        while characters.next_if(|(_, next)| *next == '\\').is_some() {}
        let Some((position, escaped)) = characters.next() else {
            break;
        };
        let end = position + escaped.len_utf8();
        match escaped {
            'n' | 'r' | 't' => folded.push(' ', start, end),
            'u' => {
                let unit = text
                    .get(end..end + 4)
                    .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                    .and_then(char::from_u32);
                match unit {
                    Some(decoded) => {
                        for _ in 0..4 {
                            characters.next();
                        }
                        folded.push(decoded, start, end + 4);
                    }
                    None => folded.push('u', start, end),
                }
            }
            other => folded.push(other, start, end),
        }
    }
    folded
}

fn fragments(quote: &str) -> Vec<String> {
    ELISIONS
        .iter()
        .fold(vec![quote.to_owned()], |parts, elision| {
            parts
                .iter()
                .flat_map(|part| part.split(elision))
                .map(str::to_owned)
                .collect()
        })
        .iter()
        .map(|part| fold(part).text.trim_matches(' ').to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

fn passages<'a>(part: &'a str, quote: &str, pieces: &[String]) -> Option<Vec<&'a str>> {
    let recorded = fold(part);
    if let Some(position) = recorded.text.find(quote) {
        return Some(vec![recorded.slice(part, position, position + quote.len())]);
    }
    let cited: usize = pieces.iter().map(String::len).sum();
    let mut cursor = 0;
    let mut matched = Vec::<(usize, usize)>::new();
    for piece in pieces
        .iter()
        .filter(|piece| piece.chars().count() >= MINIMUM_FRAGMENT)
    {
        let start = cursor + recorded.text[cursor..].find(piece.as_str())?;
        cursor = start + piece.len();
        match matched.last_mut() {
            Some(previous) if start <= previous.1 + 1 => previous.1 = cursor,
            _ => matched.push((start, cursor)),
        }
    }
    let verified: usize = matched.iter().map(|(start, end)| end - start).sum();
    let (first, last) = (matched.first()?, matched.last()?);
    if verified * 2 < cited {
        return None;
    }
    let covering = recorded.slice(part, first.0, last.1);
    if covering.len() <= quote.len() * 2 + EXCERPT_CHARACTERS {
        return Some(vec![covering]);
    }
    (matched.len() <= MAXIMUM_PASSAGES).then(|| {
        matched
            .iter()
            .map(|(start, end)| recorded.slice(part, *start, *end))
            .collect()
    })
}

pub(crate) fn recorded_passages<'a>(source: &'a str, quote: &str) -> Option<Vec<&'a str>> {
    let folded = fold(quote);
    let whole = folded.text.trim_matches(' ');
    if whole.chars().count() < MINIMUM_CHARACTERS || source.len() > u32::MAX as usize {
        return None;
    }
    let pieces = fragments(quote);
    source
        .split(OMISSION)
        .find_map(|part| passages(part, whole, &pieces))
}

pub(crate) fn closest_passage(source: &str, quote: &str) -> Option<String> {
    if source.len() > u32::MAX as usize {
        return None;
    }
    let folded = fold(quote);
    let quote = folded.text.trim_matches(' ');
    let boundaries: Vec<usize> = quote
        .char_indices()
        .map(|(index, _)| index)
        .chain([quote.len()])
        .collect();
    let recorded = fold(source);
    let position = ANCHOR_CHARACTERS
        .into_iter()
        .filter(|length| *length < boundaries.len())
        .flat_map(|length| {
            (0..boundaries.len() - length)
                .step_by(length / 2)
                .map(move |start| (start, start + length))
        })
        .find_map(|(start, end)| {
            recorded
                .text
                .find(&quote[boundaries[start]..boundaries[end]])
        })?;
    let center = recorded.starts[position] as usize;
    let start = source[..center]
        .char_indices()
        .rev()
        .nth(EXCERPT_CHARACTERS / 3)
        .map_or(0, |(index, _)| index);
    let end = source[center..]
        .char_indices()
        .nth(EXCERPT_CHARACTERS)
        .map_or(source.len(), |(index, _)| center + index);
    Some(source[start..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const TEST_LOG: &str = "Exit code 1\n======================================================================\nFAIL: test_first_page (test_paging.PagingTest.test_first_page)\n----------------------------------------------------------------------\nTraceback (most recent call last):\n  File \"/work/test_paging.py\", line 8, in test_first_page\n    self.assertEqual(page(list(range(10)), 1, 3), [0, 1, 2])\nAssertionError: Lists differ: [3, 4, 5] != [0, 1, 2]\n\nRan 3 tests in 0.001s\n\nFAILED (failures=1)";

    #[rstest]
    #[case::collapsed_whitespace(
        "exit code 1\n\n   tests  failed\tnow",
        "exit code 1 tests failed now",
        "exit code 1\n\n   tests  failed\tnow"
    )]
    #[case::recorded_escapes(
        r#"{"content":"line one\nline two \"quoted\""}"#,
        "line one\nline two \"quoted\"",
        r#"line one\nline two \"quoted\""#
    )]
    #[case::extra_escapes(
        "Exit code 1\nNo module named pytest",
        r"Exit code 1\\nNo module named pytest",
        "Exit code 1\nNo module named pytest"
    )]
    #[case::nested_escapes(
        r#"[{\"role\":\"tool\",\"content\":\"a\\nb\"}]"#,
        r#"{"role":"tool","content":"a b"}"#,
        r#"{\"role\":\"tool\",\"content\":\"a\\nb\"}"#
    )]
    #[case::emphasis(
        "the **deploy** failed with `exit 3` today",
        "the deploy failed with exit 3",
        "the **deploy** failed with `exit 3"
    )]
    #[case::typography(
        "it's \"done\" - really",
        "it\u{2019}s \u{201c}done\u{201d} \u{2014} really",
        "it's \"done\" - really"
    )]
    #[case::unicode_escape(
        r"café is closed today",
        "café is closed today",
        r"café is closed today"
    )]
    #[case::ellipsis_in_recording(
        "waiting... still waiting",
        "waiting\u{2026} still waiting",
        "waiting... still waiting"
    )]
    fn presentation_differences_resolve_to_the_recorded_text(
        #[case] source: &str,
        #[case] quote: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(recorded_passages(source, quote).unwrap(), vec![expected]);
        assert!(source.contains(expected));
    }

    #[rstest]
    #[case::dropped_lines(
        "FAIL: test_first_page (test_paging.PagingTest.test_first_page)\nAssertionError: Lists differ: [3, 4, 5] != [0, 1, 2]",
        vec!["FAIL: test_first_page (test_paging.PagingTest.test_first_page)\n----------------------------------------------------------------------\nTraceback (most recent call last):\n  File \"/work/test_paging.py\", line 8, in test_first_page\n    self.assertEqual(page(list(range(10)), 1, 3), [0, 1, 2])\nAssertionError: Lists differ: [3, 4, 5] != [0, 1, 2]"]
    )]
    #[case::marked_elision(
        "FAIL: test_first_page (test_paging.PagingTest.test_first_page) [...] AssertionError: Lists differ: [3, 4, 5] != [0, 1, 2]",
        vec!["FAIL: test_first_page (test_paging.PagingTest.test_first_page)\n----------------------------------------------------------------------\nTraceback (most recent call last):\n  File \"/work/test_paging.py\", line 8, in test_first_page\n    self.assertEqual(page(list(range(10)), 1, 3), [0, 1, 2])\nAssertionError: Lists differ: [3, 4, 5] != [0, 1, 2]"]
    )]
    #[case::short_lines_between(
        "AssertionError: Lists differ: [3, 4, 5] != [0, 1, 2]\nRan 3 tests in 0.001s\nFAILED (failures=1)",
        vec!["AssertionError: Lists differ: [3, 4, 5] != [0, 1, 2]\n\nRan 3 tests in 0.001s\n\nFAILED (failures=1)"]
    )]
    fn shortened_quotes_resolve_to_the_recorded_passage_that_covers_them(
        #[case] quote: &str,
        #[case] expected: Vec<&str>,
    ) {
        assert_eq!(recorded_passages(TEST_LOG, quote).unwrap(), expected);
    }

    #[rstest]
    fn distant_fragments_become_separate_passages() {
        let source = format!(
            "user asked to deploy billing-api\n{}deploy stopped: DEPLOY_TOKEN is not set",
            "unrelated build output line\n".repeat(200)
        );
        assert_eq!(
            recorded_passages(
                &source,
                "user asked to deploy billing-api ... deploy stopped: DEPLOY_TOKEN is not set"
            )
            .unwrap(),
            vec![
                "user asked to deploy billing-api",
                "deploy stopped: DEPLOY_TOKEN is not set"
            ]
        );
    }

    #[rstest]
    #[case::paraphrase("the tests failed with exit code 1", "the test suite did not pass")]
    #[case::reordered("alpha beta gamma delta", "gamma delta alpha beta")]
    #[case::reordered_lines(
        "first recorded line here\nsecond recorded line here",
        "second recorded line here\nfirst recorded line here"
    )]
    #[case::invented_line(
        "first recorded line here\nsecond recorded line here",
        "first recorded line here\nthis line was never recorded"
    )]
    #[case::mostly_unverified(
        "Traceback (most recent call last):\nValueError",
        "Traceback (most recent call last):\nboom\nbang\nNo such\nfile or\ndirectory\nexit 9\nstatus 1"
    )]
    #[case::too_short("status ok", "ok")]
    #[case::only_markup("status ok", "\\n ** ``")]
    #[case::joined_across_omission(
        "first half of the text\n[... content omitted ...]\nsecond half of the text",
        "first half of the text\nsecond half of the text"
    )]
    #[case::different_case("Deploy Failed Today", "deploy failed today")]
    fn different_text_is_never_accepted(#[case] source: &str, #[case] quote: &str) {
        assert_eq!(recorded_passages(source, quote), None);
    }

    #[rstest]
    fn rejected_quote_gets_the_recorded_passage_it_resembles() {
        let source = format!(
            "{}ERROR: DEPLOY_TOKEN is not set; refusing to push to production registry{}",
            "build log line\n".repeat(200),
            "\ntrailing output".repeat(200)
        );
        let excerpt =
            closest_passage(&source, "DEPLOY_TOKEN is not set, so the push was refused").unwrap();
        assert!(excerpt.contains("ERROR: DEPLOY_TOKEN is not set; refusing to push"));
        assert!(excerpt.chars().count() <= EXCERPT_CHARACTERS + EXCERPT_CHARACTERS / 3 + 1);
        assert!(source.contains(&excerpt));
    }

    #[rstest]
    fn unrelated_quote_has_no_recorded_passage() {
        assert_eq!(
            closest_passage(
                "build log line\nall good",
                "the database migration was rolled back"
            ),
            None
        );
    }
}
