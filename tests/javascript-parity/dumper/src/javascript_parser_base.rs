use antlr4_runtime::{HIDDEN_CHANNEL, ParserSemCtx, Token, TokenSource};

use crate::generated::java_script_parser::{
    CLOSE_BRACE, FUNCTION, JavaScriptParserHooks, LINE_TERMINATOR, MULTI_LINE_COMMENT, OPEN_BRACE,
    WHITE_SPACES,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct JavaScriptParserBase;

impl JavaScriptParserBase {
    fn raw_token<S>(ctx: &mut ParserSemCtx<'_, S>, index: usize) -> Option<(i32, i32, String)>
    where
        S: TokenSource,
    {
        ctx.token_at(index).map(|token| {
            (
                token.channel(),
                token.token_type(),
                token.text_or_empty().to_owned(),
            )
        })
    }

    fn has_line_terminator_ahead<S>(ctx: &mut ParserSemCtx<'_, S>) -> bool
    where
        S: TokenSource,
    {
        let current = ctx.input_index();
        let Some(previous) = current.checked_sub(1) else {
            return false;
        };
        let Some((channel, mut token_type, mut text)) = Self::raw_token(ctx, previous) else {
            return false;
        };
        if channel != HIDDEN_CHANNEL {
            return false;
        }
        if token_type == LINE_TERMINATOR {
            return true;
        }
        if token_type == WHITE_SPACES {
            let Some(before_whitespace) = previous.checked_sub(1) else {
                return false;
            };
            let Some((_, next_type, next_text)) = Self::raw_token(ctx, before_whitespace) else {
                return false;
            };
            token_type = next_type;
            text = next_text;
        }
        token_type == LINE_TERMINATOR
            || (token_type == MULTI_LINE_COMMENT && (text.contains('\r') || text.contains('\n')))
    }
}

impl JavaScriptParserHooks for JavaScriptParserBase {
    fn n<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>, expected: &str) -> bool
    where
        S: TokenSource,
    {
        ctx.token_text(1)
            .is_some_and(|token| token.text() == Some(expected))
    }

    fn not_line_terminator<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>) -> bool
    where
        S: TokenSource,
    {
        !Self::has_line_terminator_ahead(ctx)
    }

    fn line_terminator_ahead<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>) -> bool
    where
        S: TokenSource,
    {
        Self::has_line_terminator_ahead(ctx)
    }

    fn not_open_brace_and_not_function<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>) -> bool
    where
        S: TokenSource,
    {
        !matches!(ctx.la(1), OPEN_BRACE | FUNCTION)
    }

    fn close_brace<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>) -> bool
    where
        S: TokenSource,
    {
        ctx.la(1) == CLOSE_BRACE
    }
}
