use crate::evidence::character_range;
use unicode_casefold::UnicodeCaseFold;

pub(crate) struct Matcher {
    needle: String,
    length: usize,
    tail: String,
    literal: bool,
    matched: bool,
}

impl Matcher {
    pub(crate) fn new(needle: &str, literal: bool) -> Self {
        let needle = if literal {
            needle.to_owned()
        } else {
            needle.case_fold().collect()
        };
        Self {
            length: needle.chars().count(),
            needle,
            tail: String::new(),
            literal,
            matched: false,
        }
    }

    pub(crate) fn push(&mut self, piece: &str) {
        if self.matched || self.needle.is_empty() {
            return;
        }
        let marker = "\n[... content omitted ...]\n";
        let delay = if self.literal { marker.len() - 1 } else { 0 };
        let text = if self.literal {
            std::mem::take(&mut self.tail) + piece
        } else {
            std::mem::take(&mut self.tail) + &piece.case_fold().collect::<String>()
        };
        let segments: Vec<&str> = if self.literal {
            text.split(marker).collect()
        } else {
            vec![&text]
        };
        let last = segments[segments.len() - 1];
        let count = last.chars().count();
        self.matched = segments[..segments.len() - 1]
            .iter()
            .any(|s| s.contains(&self.needle))
            || character_range(last, 0, Some(count.saturating_sub(delay))).contains(&self.needle);
        self.tail = character_range(last, count.saturating_sub(self.length - 1 + delay), None);
    }

    pub(crate) fn matched(&self) -> bool {
        self.matched
    }

    pub(crate) fn finish(&self) -> bool {
        if self.needle.is_empty() {
            return !self.literal;
        }
        self.matched || self.tail.contains(&self.needle)
    }
}
