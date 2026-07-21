use antlr4_runtime::{
    CharStream, INVALID_TOKEN_TYPE, LexerCustomAction, LexerLifecycleCtx, LexerSemCtx,
    SemanticHooks, TokenView,
};

use super::generated::antlr_v4_lexer::{
    ACTION, ARGUMENT_CONTENT, AT, AntlRv4LexerHooks, AntlRv4LexerTypedHooks, CHANNELS, ID, OPTIONS,
    RBRACE, RULE_REF, SEMI, TOKEN_REF, TOKENS,
};

const PREQUEL_CONSTRUCT: i32 = -10;
const OPTIONS_CONSTRUCT: i32 = -11;
const ARGUMENT_MODE: i32 = 1;
const LEXER_CHAR_SET_MODE: i32 = 2;

#[derive(Clone, Debug)]
struct LexerAdaptorState {
    current_rule_type: i32,
    enclosing_rule_type: Option<i32>,
}

impl Default for LexerAdaptorState {
    fn default() -> Self {
        Self {
            current_rule_type: INVALID_TOKEN_TYPE,
            enclosing_rule_type: None,
        }
    }
}

impl AntlRv4LexerHooks for LexerAdaptorState {
    fn handle_begin_argument<I>(&mut self, ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        if self.current_rule_type == TOKEN_REF {
            ctx.push_mode(LEXER_CHAR_SET_MODE);
            ctx.more();
        } else {
            ctx.push_mode(ARGUMENT_MODE);
        }
    }

    fn handle_end_argument<I>(&mut self, ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        if ctx.pop_mode() == Some(ARGUMENT_MODE) {
            ctx.set_type(ARGUMENT_CONTENT);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct LexerAdaptor(AntlRv4LexerTypedHooks<LexerAdaptorState>);

impl SemanticHooks for LexerAdaptor {
    fn observes_parser_predicates(&self) -> bool {
        false
    }

    fn lexer_sempred<I>(
        &mut self,
        ctx: &mut LexerSemCtx<'_, I>,
        rule_index: usize,
        pred_index: usize,
    ) -> Option<bool>
    where
        I: CharStream,
    {
        self.0.lexer_sempred(ctx, rule_index, pred_index)
    }

    fn lexer_action<I>(&mut self, ctx: &mut LexerSemCtx<'_, I>, action: LexerCustomAction) -> bool
    where
        I: CharStream,
    {
        self.0.lexer_action(ctx, action)
    }

    fn lexer_reset<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
    where
        I: CharStream,
    {
        self.0.0.current_rule_type = INVALID_TOKEN_TYPE;
        self.0.0.enclosing_rule_type = None;
        self.0.lexer_reset(ctx);
    }

    fn lexer_before_token<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
    where
        I: CharStream,
    {
        self.0.lexer_before_token(ctx);
    }

    fn lexer_after_accept<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
    where
        I: CharStream,
    {
        self.0.lexer_after_accept(ctx);

        let state = &mut self.0.0;
        let token_type = ctx.token_type();
        if matches!(token_type, OPTIONS | TOKENS | CHANNELS)
            && state.current_rule_type == INVALID_TOKEN_TYPE
        {
            state.current_rule_type = PREQUEL_CONSTRUCT;
        } else if token_type == OPTIONS && matches!(state.current_rule_type, RULE_REF | TOKEN_REF) {
            state.enclosing_rule_type = Some(state.current_rule_type);
            state.current_rule_type = OPTIONS_CONSTRUCT;
        } else if token_type == RBRACE && state.current_rule_type == PREQUEL_CONSTRUCT {
            state.current_rule_type = INVALID_TOKEN_TYPE;
        } else if token_type == RBRACE && state.current_rule_type == OPTIONS_CONSTRUCT {
            state.current_rule_type = state
                .enclosing_rule_type
                .take()
                .unwrap_or(INVALID_TOKEN_TYPE);
        } else if token_type == AT && state.current_rule_type == INVALID_TOKEN_TYPE {
            state.current_rule_type = AT;
        } else if token_type == SEMI && state.current_rule_type == OPTIONS_CONSTRUCT {
            // The option terminator does not end the surrounding rule.
        } else if token_type == ACTION && state.current_rule_type == AT {
            state.current_rule_type = INVALID_TOKEN_TYPE;
        } else if token_type == ID {
            let text = ctx.token_text();
            let first = text.chars().next().expect("ID tokens are non-empty");
            let classified = if first.is_uppercase() {
                TOKEN_REF
            } else {
                RULE_REF
            };
            ctx.set_type(classified);
            if state.current_rule_type == INVALID_TOKEN_TYPE {
                state.current_rule_type = classified;
            }
        } else if token_type == SEMI {
            state.current_rule_type = INVALID_TOKEN_TYPE;
        }
    }

    fn lexer_token_emitted(&mut self, token: TokenView<'_>) {
        self.0.lexer_token_emitted(token);
    }
}

#[cfg(test)]
mod tests {
    use super::super::frontend::{SourceId, parse_source};
    use super::super::generated::antlr_v4_lexer::AntlRv4Lexer;
    use super::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Token};

    #[test]
    fn grammar_options_preserve_following_lexer_rule_mode() {
        let grammar = concat!(
            "lexer grammar G;\n",
            "options { language=Rust; tokenVocab=T; }\n",
            "A: [a];\n",
        );

        parse_source(SourceId::new(0), "memory:grammar-options", grammar)
            .expect("grammar options must not turn a later lexer charset into arguments");
    }

    #[test]
    fn parser_rule_options_preserve_argument_mode() {
        let grammar = concat!(
            "grammar G;\n",
            "a options { k=1; } : T b[0];\n",
            "b[int value] : T;\n",
            "T: 'x';\n",
        );

        parse_source(SourceId::new(0), "memory:parser-rule-options", grammar)
            .expect("parser-rule options must not turn later arguments into lexer char sets");
    }

    #[test]
    fn uncased_initials_are_rule_references() {
        let lexer = AntlRv4Lexer::with_hooks(
            InputStream::new("文: 'x'; ÄToken: 'z';"),
            LexerAdaptor::default(),
        );
        let tokens = CommonTokenStream::new(lexer);
        let references = tokens
            .tokens()
            .filter(|token| matches!(token.token_type(), RULE_REF | TOKEN_REF))
            .map(|token| (token.text().to_owned(), token.token_type()))
            .collect::<Vec<_>>();

        assert_eq!(
            references,
            [
                ("文".to_owned(), RULE_REF),
                ("ÄToken".to_owned(), TOKEN_REF),
            ]
        );
    }
}
