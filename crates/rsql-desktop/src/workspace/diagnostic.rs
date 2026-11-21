pub(super) fn bounded_message(message: impl std::fmt::Display) -> String {
    use std::fmt::{self, Write};

    const MAX_BYTES: usize = 1024;
    struct Diagnostic(String);
    impl Write for Diagnostic {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            let remaining = MAX_BYTES - self.0.len();
            if text.len() <= remaining {
                self.0.push_str(text);
                return Ok(());
            }
            self.0
                .push_str(&text[..text.floor_char_boundary(remaining)]);
            self.0
                .truncate(self.0.floor_char_boundary(MAX_BYTES - '…'.len_utf8()));
            self.0.push('…');
            Err(fmt::Error)
        }
    }

    let mut output = Diagnostic(String::new());
    let _ = write!(output, "{message}");
    output.0
}
