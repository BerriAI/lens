type Failure = (usize, String);

pub(super) fn diagnose(body: &[u8]) -> Option<Failure> {
    let mut parser = Parser {
        text: String::from_utf8_lossy(body).chars().collect(),
        offset: 0,
    };
    match parser.value(0) {
        Err(error) => Some(error),
        Ok(()) => {
            parser.whitespace();
            (parser.offset < parser.text.len()).then(|| parser.error("Extra data"))
        }
    }
}

struct Parser {
    text: Vec<char>,
    offset: usize,
}

impl Parser {
    fn current(&self) -> Option<char> {
        self.text.get(self.offset).copied()
    }
    fn error(&self, message: &str) -> Failure {
        (self.offset, message.to_owned())
    }
    fn whitespace(&mut self) {
        while matches!(self.current(), Some(' ' | '\t' | '\r' | '\n')) {
            self.offset += 1;
        }
    }
    fn value(&mut self, depth: usize) -> Result<(), Failure> {
        self.whitespace();
        if depth > 128 {
            return Err(self.error("JSON nesting exceeds the supported depth"));
        }
        match self.current() {
            Some('"') => self.string(),
            Some('{') => self.object(depth + 1),
            Some('[') => self.array(depth + 1),
            Some('t') => self.literal("true"),
            Some('f') => self.literal("false"),
            Some('n') => self.literal("null"),
            Some('-' | '0'..='9') => self.number(),
            _ => Err(self.error("Expecting value")),
        }
    }
    fn literal(&mut self, literal: &str) -> Result<(), Failure> {
        if literal
            .chars()
            .enumerate()
            .all(|(index, char)| self.text.get(self.offset + index) == Some(&char))
        {
            self.offset += literal.len();
            Ok(())
        } else {
            Err(self.error("Expecting value"))
        }
    }
    fn string(&mut self) -> Result<(), Failure> {
        let start = self.offset;
        self.offset += 1;
        while let Some(char) = self.current() {
            match char {
                '"' => {
                    self.offset += 1;
                    return Ok(());
                }
                '\\' => {
                    let escape = self.offset;
                    self.offset += 1;
                    match self.current() {
                        Some('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't') => self.offset += 1,
                        Some('u') => {
                            let unicode = self.offset;
                            self.offset += 1;
                            for _ in 0..4 {
                                if !self.current().is_some_and(|char| char.is_ascii_hexdigit()) {
                                    return Err((unicode, "Invalid \\uXXXX escape".to_owned()));
                                }
                                self.offset += 1;
                            }
                        }
                        None => return Err((start, "Unterminated string starting at".to_owned())),
                        _ => return Err((escape, "Invalid \\escape".to_owned())),
                    }
                }
                '\0'..='\u{1f}' => return Err(self.error("Invalid control character at")),
                _ => self.offset += 1,
            }
        }
        Err((start, "Unterminated string starting at".to_owned()))
    }
    fn object(&mut self, depth: usize) -> Result<(), Failure> {
        self.offset += 1;
        self.whitespace();
        if self.current() == Some('}') {
            self.offset += 1;
            return Ok(());
        }
        loop {
            if self.current() != Some('"') {
                return Err(self.error("Expecting property name enclosed in double quotes"));
            }
            self.string()?;
            self.whitespace();
            if self.current() != Some(':') {
                return Err(self.error("Expecting ':' delimiter"));
            }
            self.offset += 1;
            self.value(depth)?;
            self.whitespace();
            if self.current() == Some('}') {
                self.offset += 1;
                return Ok(());
            }
            if self.current() != Some(',') {
                return Err(self.error("Expecting ',' delimiter"));
            }
            let comma = self.offset;
            self.offset += 1;
            self.whitespace();
            if self.current() == Some('}') {
                return Err((
                    comma,
                    "Illegal trailing comma before end of object".to_owned(),
                ));
            }
        }
    }
    fn array(&mut self, depth: usize) -> Result<(), Failure> {
        self.offset += 1;
        self.whitespace();
        if self.current() == Some(']') {
            self.offset += 1;
            return Ok(());
        }
        loop {
            self.value(depth)?;
            self.whitespace();
            if self.current() == Some(']') {
                self.offset += 1;
                return Ok(());
            }
            if self.current() != Some(',') {
                return Err(self.error("Expecting ',' delimiter"));
            }
            let comma = self.offset;
            self.offset += 1;
            self.whitespace();
            if self.current() == Some(']') {
                return Err((
                    comma,
                    "Illegal trailing comma before end of array".to_owned(),
                ));
            }
        }
    }
    fn number(&mut self) -> Result<(), Failure> {
        let start = self.offset;
        if self.current() == Some('-') {
            self.offset += 1;
        }
        match self.current() {
            Some('0') => self.offset += 1,
            Some('1'..='9') => {
                while self.current().is_some_and(|char| char.is_ascii_digit()) {
                    self.offset += 1;
                }
            }
            _ => return Err((start, "Expecting value".to_owned())),
        }
        if self.current() == Some('.')
            && self
                .text
                .get(self.offset + 1)
                .is_some_and(char::is_ascii_digit)
        {
            self.offset += 1;
            while self.current().is_some_and(|char| char.is_ascii_digit()) {
                self.offset += 1;
            }
        }
        if matches!(self.current(), Some('e' | 'E')) {
            let exponent = self.offset;
            self.offset += 1;
            if matches!(self.current(), Some('+' | '-')) {
                self.offset += 1;
            }
            if !self.current().is_some_and(|char| char.is_ascii_digit()) {
                self.offset = exponent;
                return Ok(());
            }
            while self.current().is_some_and(|char| char.is_ascii_digit()) {
                self.offset += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::diagnose;
    use rstest::rstest;

    #[rstest]
    #[case::empty("", 0, "Expecting value")]
    #[case::object("{", 1, "Expecting property name enclosed in double quotes")]
    #[case::multiline("{\n  \"name\": ", 12, "Expecting value")]
    #[case::literal("{\"name\": tru}", 9, "Expecting value")]
    #[case::unterminated("{\"name\":\"hi}", 8, "Unterminated string starting at")]
    #[case::unicode_position("{\"name\":\"é\"} trailing", 13, "Extra data")]
    #[case::array_comma("{\"name\": [1,]}", 11, "Illegal trailing comma before end of array")]
    #[case::object_comma("{\"a\":1,}", 6, "Illegal trailing comma before end of object")]
    #[case::leading_zero("{\"name\": 01}", 10, "Expecting ',' delimiter")]
    #[case::escape("{\"name\":\"a\\q\"}", 10, "Invalid \\escape")]
    #[case::short_array("[1", 2, "Expecting ',' delimiter")]
    #[case::missing_array_value("[1,", 3, "Expecting value")]
    #[case::colon("{\"a\"", 4, "Expecting ':' delimiter")]
    #[case::unicode_escape("{\"a\":\"\\u12x4\"}", 7, "Invalid \\uXXXX escape")]
    #[case::unterminated_escape("{\"a\":\"\\", 5, "Unterminated string starting at")]
    #[case::control("{\"a\":\"\n\"}", 6, "Invalid control character at")]
    #[case::exponent("{\"a\":1e}", 6, "Expecting ',' delimiter")]
    #[case::minus("{\"a\":-}", 5, "Expecting value")]
    fn python_json_error_positions_and_messages_are_preserved(
        #[case] body: &str,
        #[case] position: usize,
        #[case] message: &str,
    ) {
        assert_eq!(
            diagnose(body.as_bytes()),
            Some((position, message.to_owned()))
        );
    }

    #[rstest]
    #[case::nested(r#"{"a":[true,false,null,-0.2e+3,"\u1234\t"],"b":{}}"#)]
    #[case::empty_array("[]")]
    fn valid_json_does_not_invent_an_error(#[case] body: &str) {
        assert_eq!(diagnose(body.as_bytes()), None);
    }
}
