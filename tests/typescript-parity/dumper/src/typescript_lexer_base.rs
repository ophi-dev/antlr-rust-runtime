use antlr4_runtime::{CharStream, DEFAULT_CHANNEL, LexerSemCtx, Token, TokenView};

use crate::generated::type_script_lexer::{
    BOOLEAN_LITERAL, CLOSE_BRACKET, CLOSE_PAREN, DECIMAL_LITERAL, HEX_INTEGER_LITERAL, IDENTIFIER,
    MINUS_MINUS, NULL_LITERAL, OCTAL_INTEGER_LITERAL, OPEN_BRACE, PLUS_PLUS, STRING_LITERAL, THIS,
    TypeScriptLexerHooks,
};

#[derive(Clone, Debug, Default)]
pub struct TypeScriptLexerBase {
    scope_strict_modes: Vec<bool>,
    last_token_type: Option<i32>,
    use_strict_default: bool,
    use_strict_current: bool,
    template_depth: i32,
    braces_depth: i32,
}

impl TypeScriptLexerBase {
    #[must_use]
    pub fn with_strict_default(value: bool) -> Self {
        Self {
            use_strict_default: value,
            use_strict_current: value,
            ..Self::default()
        }
    }

    fn push_strict_mode_scope(&mut self, value: bool) {
        self.scope_strict_modes.push(value);
    }

    fn pop_strict_mode_scope(&mut self) -> Option<bool> {
        self.scope_strict_modes.pop()
    }
}

impl TypeScriptLexerHooks for TypeScriptLexerBase {
    fn is_strict_mode<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>) -> bool
    where
        I: CharStream,
    {
        self.use_strict_current
    }

    fn is_regex_possible<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>) -> bool
    where
        I: CharStream,
    {
        !matches!(
            self.last_token_type,
            Some(
                IDENTIFIER
                    | NULL_LITERAL
                    | BOOLEAN_LITERAL
                    | THIS
                    | CLOSE_BRACKET
                    | CLOSE_PAREN
                    | OCTAL_INTEGER_LITERAL
                    | DECIMAL_LITERAL
                    | HEX_INTEGER_LITERAL
                    | STRING_LITERAL
                    | PLUS_PLUS
                    | MINUS_MINUS
            )
        )
    }

    fn is_in_template_string<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>) -> bool
    where
        I: CharStream,
    {
        self.template_depth > 0 && self.braces_depth == 0
    }

    fn process_open_brace<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        self.braces_depth += 1;
        self.use_strict_current =
            self.scope_strict_modes.last().copied().unwrap_or(false) || self.use_strict_default;
        self.push_strict_mode_scope(self.use_strict_current);
    }

    fn process_close_brace<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        self.braces_depth -= 1;
        self.use_strict_current = self
            .pop_strict_mode_scope()
            .unwrap_or(self.use_strict_default);
    }

    fn process_string_literal<I>(&mut self, ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        if self.last_token_type.is_none() || self.last_token_type == Some(OPEN_BRACE) {
            let text = ctx.text_so_far();
            if text == "\"use strict\"" || text == "'use strict'" {
                let _ = self.pop_strict_mode_scope();
                self.use_strict_current = true;
                self.push_strict_mode_scope(true);
            }
        }
    }

    fn start_template_string<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        self.braces_depth = 0;
    }

    fn increase_template_depth<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        self.template_depth += 1;
    }

    fn decrease_template_depth<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        self.template_depth -= 1;
    }

    fn token_emitted(&mut self, token: TokenView<'_>) {
        if token.channel() == DEFAULT_CHANNEL {
            self.last_token_type = Some(token.token_type());
        }
    }
}
