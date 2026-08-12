// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
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
