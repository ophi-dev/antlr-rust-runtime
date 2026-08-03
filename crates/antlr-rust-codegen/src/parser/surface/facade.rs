/// Token-stream facade matching the `.test.stg` surface
/// (`self.input().text()` / `.la(i)` / `.lt(i).text()`).
const EMBEDDED_INPUT_FACADE: &str = r#"
#[allow(dead_code)]
pub struct __GeneratedInput<'a, L: TokenSource>(&'a mut CommonTokenStream<L>);

#[allow(dead_code)]
impl<L: TokenSource> __GeneratedInput<'_, L> {
    pub fn text(&mut self) -> String {
        self.0.text_all()
    }

    pub fn la(&mut self, offset: isize) -> i32 {
        antlr4_runtime::IntStream::la(self.0, offset)
    }

    pub fn lt(&mut self, offset: isize) -> __GeneratedTokenView {
        __GeneratedTokenView {
            text: self
                .0
                .lt(offset)
                .map(|token| token.text_or_empty().to_owned())
                .unwrap_or_default(),
        }
    }
}

#[allow(dead_code)]
pub struct __GeneratedTokenView {
    text: String,
}

#[allow(dead_code)]
impl __GeneratedTokenView {
    pub fn text(&self) -> &str {
        &self.text
    }
}
"#;

fn render_antlr4rust_input_facade(input_facade: &str, token_view: &str) -> String {
    let mut out = String::with_capacity(ANTLR4RUST_INPUT_FACADE_TEMPLATE.len());
    let mut rest = ANTLR4RUST_INPUT_FACADE_TEMPLATE;
    loop {
        let next = [
            rest.find(embedded::ANTLR4RUST_INPUT_FACADE)
                .map(|index| (index, embedded::ANTLR4RUST_INPUT_FACADE, input_facade)),
            rest.find(embedded::ANTLR4RUST_TOKEN_VIEW)
                .map(|index| (index, embedded::ANTLR4RUST_TOKEN_VIEW, token_view)),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|(index, _, _)| *index);
        let Some((index, needle, replacement)) = next else {
            break;
        };
        out.push_str(&rest[..index]);
        out.push_str(replacement);
        rest = &rest[index + needle.len()..];
    }
    out.push_str(rest);
    out
}

const ANTLR4RUST_INPUT_FACADE_TEMPLATE: &str = r#"
/// Borrowed parser-input facade for embedded bodies produced by antlr4rust
/// grammar transforms.
#[allow(dead_code)]
struct __Antlr4RustInput<'a, L: TokenSource>(&'a CommonTokenStream<L>);

#[allow(dead_code)]
impl<'a, L: TokenSource> __Antlr4RustInput<'a, L> {
    fn la(&self, offset: isize) -> i32 {
        self.lt(offset).map_or(antlr4_runtime::INVALID_TOKEN_TYPE, |token| {
            token.get_token_type()
        })
    }

    fn lt(&self, offset: isize) -> Option<__Antlr4RustTokenView<'a>> {
        let token = self.0.lt(offset).or_else(|| {
            // ANTLR clamps every positive past-end request to the buffered EOF token.
            if offset > 0 {
                self.0.get(self.0.token_count().saturating_sub(1))
            } else {
                None
            }
        });
        token.map(__Antlr4RustTokenView)
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
struct __Antlr4RustTokenView<'a>(antlr4_runtime::TokenView<'a>);

#[allow(dead_code)]
impl<'a> __Antlr4RustTokenView<'a> {
    fn get_text(&self) -> &'a str {
        self.0.text_or_empty()
    }

    fn get_token_type(&self) -> i32 {
        self.0.token_type()
    }
}
"#;
