use antlr4_runtime::{HIDDEN_CHANNEL, ParserSemCtx, Token, TokenSource};

use crate::generated::type_script_parser::{
    CLOSE_BRACE, FUNCTION, INTERFACE, LINE_TERMINATOR, MULTI_LINE_COMMENT, OPEN_BRACE,
    TypeScriptParserHooks, WHITE_SPACES,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct TypeScriptParserBase;

impl TypeScriptParserBase {
    fn raw_token<S>(ctx: &mut ParserSemCtx<'_, S>, index: usize) -> Option<(i32, i32, String)>
    where
        S: TokenSource,
    {
        ctx.token_at(index)
            .map(|token| (token.channel(), token.token_type(), token.text().to_owned()))
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

    fn immediate_hidden_token_is<S>(ctx: &mut ParserSemCtx<'_, S>, token_type: i32) -> bool
    where
        S: TokenSource,
    {
        let Some(previous) = ctx.input_index().checked_sub(1) else {
            return false;
        };
        Self::raw_token(ctx, previous)
            .is_some_and(|(channel, actual, _)| channel == HIDDEN_CHANNEL && actual == token_type)
    }
}

impl TypeScriptParserHooks for TypeScriptParserBase {
    fn p<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>, expected: &str) -> bool
    where
        S: TokenSource,
    {
        ctx.token_text(-1)
            .is_some_and(|token| token.text() == expected)
    }

    fn n<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>, expected: &str) -> bool
    where
        S: TokenSource,
    {
        ctx.token_text(1)
            .is_some_and(|token| token.text() == expected)
    }

    fn not_line_terminator<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>) -> bool
    where
        S: TokenSource,
    {
        !Self::immediate_hidden_token_is(ctx, LINE_TERMINATOR)
    }

    fn line_terminator_ahead<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>) -> bool
    where
        S: TokenSource,
    {
        Self::has_line_terminator_ahead(ctx)
    }

    fn not_open_brace_and_not_function_and_not_interface<S>(
        &mut self,
        ctx: &mut ParserSemCtx<'_, S>,
    ) -> bool
    where
        S: TokenSource,
    {
        !matches!(ctx.la(1), OPEN_BRACE | FUNCTION | INTERFACE)
    }

    fn close_brace<S>(&mut self, ctx: &mut ParserSemCtx<'_, S>) -> bool
    where
        S: TokenSource,
    {
        ctx.la(1) == CLOSE_BRACE
    }
}
