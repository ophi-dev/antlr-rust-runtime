use std::collections::BTreeSet;

use antlr4_runtime::{CharStream, EOF, HIDDEN_CHANNEL, LexerLifecycleCtx, LexerSemCtx};

use crate::generated::c_sharp_lexer::{
    BANG, CHANNEL_COMMENTS_CHANNEL, CHANNEL_DIRECTIVE, CLOSE_PARENS, CONDITIONAL_SYMBOL,
    CSharpLexerHooks, DEFINE, DIRECTIVE_NEW_LINE, ELIF, ELSE, ENDIF, FALSE, IF,
    MODE_INTERPOLATION_FORMAT, OP_AND, OP_EQ, OP_NE, OP_OR, OPEN_PARENS, SKIPPED_SECTION, TRUE,
    UNDEF,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct CSharpLexerBase {
    interpolated_string_level: usize,
    interpolated_verbatiums: Vec<bool>,
    curly_levels: Vec<usize>,
    verbatium: bool,
    symbols: BTreeSet<String>,
    conditions: Vec<bool>,
    taken: Vec<bool>,
    directive: Option<Directive>,
}

#[derive(Clone, Debug)]
struct Directive {
    kind: i32,
    tokens: Vec<(i32, String)>,
}

impl CSharpLexerBase {
    fn is_active(&self) -> bool {
        self.conditions.last().copied().unwrap_or(true)
    }

    fn finish_directive<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
    where
        I: CharStream,
    {
        let Some(directive) = self.directive.take() else {
            return;
        };
        let mut skip_inactive = false;
        match directive.kind {
            DEFINE => {
                if self.is_active()
                    && let Some(symbol) = conditional_symbol(&directive.tokens)
                {
                    self.symbols.insert(symbol.to_owned());
                }
            }
            UNDEF => {
                if self.is_active()
                    && let Some(symbol) = conditional_symbol(&directive.tokens)
                {
                    self.symbols.remove(symbol);
                }
            }
            IF => {
                let outer = self.is_active();
                let result = outer && evaluate(&directive.tokens, &self.symbols);
                self.conditions.push(result);
                self.taken.push(result);
                skip_inactive = !result;
            }
            ELIF => {
                let already_taken = self.taken.pop().unwrap_or(false);
                let _ = self.conditions.pop();
                let outer = self.is_active();
                let result = !already_taken && outer && evaluate(&directive.tokens, &self.symbols);
                self.conditions.push(result);
                self.taken.push(already_taken || result);
                skip_inactive = !result;
            }
            ELSE => {
                let already_taken = self.taken.pop().unwrap_or(false);
                let _ = self.conditions.pop();
                let result = !already_taken && self.is_active();
                self.conditions.push(result);
                self.taken.push(true);
                skip_inactive = !result;
            }
            ENDIF => {
                let _ = self.conditions.pop();
                let _ = self.taken.pop();
            }
            _ => {}
        }
        if skip_inactive {
            skip_false_block(ctx);
        }
    }
}

impl CSharpLexerHooks for CSharpLexerBase {
    fn lexer_reset<I>(&mut self, _ctx: &mut LexerLifecycleCtx<'_, I>)
    where
        I: CharStream,
    {
        *self = Self::default();
    }

    fn lexer_before_token<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
    where
        I: CharStream,
    {
        if self.directive.is_some() && ctx.la(1) == EOF {
            self.finish_directive(ctx);
        }
    }

    fn lexer_after_accept<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
    where
        I: CharStream,
    {
        let token_type = ctx.token_type();
        if self.directive.is_none()
            && ctx.channel() == CHANNEL_DIRECTIVE
            && matches!(token_type, DEFINE | UNDEF | IF | ELIF | ELSE | ENDIF)
        {
            self.directive = Some(Directive {
                kind: token_type,
                tokens: Vec::new(),
            });
            return;
        }

        let Some(directive) = self.directive.as_mut() else {
            return;
        };
        if token_type == DIRECTIVE_NEW_LINE {
            ctx.skip();
            self.finish_directive(ctx);
            return;
        }
        if !matches!(ctx.channel(), HIDDEN_CHANNEL | CHANNEL_COMMENTS_CHANNEL) {
            directive
                .tokens
                .push((token_type, ctx.accepted_text().unwrap_or_default()));
        }
        ctx.skip();
    }

    fn is_regular_char_inside<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>) -> bool
    where
        I: CharStream,
    {
        !self.verbatium
    }

    fn is_verbatium_double_quote_inside<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>) -> bool
    where
        I: CharStream,
    {
        self.verbatium
    }

    fn on_interpolated_regular_string_start<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        self.interpolated_string_level += 1;
        self.interpolated_verbatiums.push(false);
        self.verbatium = false;
    }

    fn on_interpolated_verbatium_string_start<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        self.interpolated_string_level += 1;
        self.interpolated_verbatiums.push(true);
        self.verbatium = true;
    }

    fn on_open_brace<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        if self.interpolated_string_level > 0
            && let Some(level) = self.curly_levels.last_mut()
        {
            *level += 1;
        }
    }

    fn on_close_brace<I>(&mut self, ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        if self.interpolated_string_level == 0 {
            return;
        }
        let Some(level) = self.curly_levels.last_mut() else {
            return;
        };
        *level = level.saturating_sub(1);
        if *level == 0 {
            let _ = self.curly_levels.pop();
            ctx.skip();
            let _ = ctx.pop_mode();
        }
    }

    fn on_colon<I>(&mut self, ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        if self.interpolated_string_level == 0 {
            return;
        }
        let mut offset = 1;
        let mut switch_to_format = true;
        loop {
            match ctx.la(offset) {
                value if value == '}' as i32 || value == EOF => break,
                value if value == ':' as i32 || value == ')' as i32 => {
                    switch_to_format = false;
                    break;
                }
                _ => offset += 1,
            }
        }
        if switch_to_format {
            ctx.set_mode(MODE_INTERPOLATION_FORMAT);
        }
    }

    fn open_brace_inside<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        self.curly_levels.push(1);
    }

    fn on_double_quote_inside<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        self.interpolated_string_level = self.interpolated_string_level.saturating_sub(1);
        let _ = self.interpolated_verbatiums.pop();
        self.verbatium = self
            .interpolated_verbatiums
            .last()
            .copied()
            .unwrap_or(false);
    }

    fn on_close_brace_inside<I>(&mut self, _ctx: &mut LexerSemCtx<'_, I>)
    where
        I: CharStream,
    {
        let _ = self.curly_levels.pop();
    }
}

fn conditional_symbol(tokens: &[(i32, String)]) -> Option<&str> {
    tokens
        .iter()
        .find_map(|(kind, text)| (*kind == CONDITIONAL_SYMBOL).then_some(text.as_str()))
}

fn skip_false_block<I>(ctx: &mut LexerLifecycleCtx<'_, I>)
where
    I: CharStream,
{
    let start = ctx.input_position();
    let _ = ctx.set_token_start(start);
    let mut depth = 1_usize;
    let mut at_line_start = true;
    loop {
        let ch = ctx.la(1);
        if ch == EOF {
            break;
        }
        if is_newline(ch) {
            ctx.consume();
            if ch == '\r' as i32 && ctx.la(1) == '\n' as i32 {
                ctx.consume();
            }
            at_line_start = true;
            continue;
        }
        if at_line_start && matches!(ch, value if value == ' ' as i32 || value == '\t' as i32) {
            ctx.consume();
            continue;
        }
        if at_line_start && ch == '#' as i32 {
            match peek_keyword(ctx).as_str() {
                "if" => depth += 1,
                "endif" => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                "else" | "elif" if depth == 1 => break,
                _ => {}
            }
        }
        at_line_start = false;
        ctx.consume();
    }
    let stop = ctx.input_position().saturating_sub(1);
    if start <= stop {
        ctx.enqueue_token_with_channel(SKIPPED_SECTION, HIDDEN_CHANNEL, stop);
    }
}

fn peek_keyword<I>(ctx: &mut LexerLifecycleCtx<'_, I>) -> String
where
    I: CharStream,
{
    let mut offset = 2_isize;
    while matches!(ctx.la(offset), value if value == ' ' as i32 || value == '\t' as i32) {
        offset += 1;
    }
    let mut keyword = String::new();
    loop {
        let ch = ctx.la(offset);
        let Some(ch) = u32::try_from(ch).ok().and_then(char::from_u32) else {
            break;
        };
        if !ch.is_alphabetic() {
            break;
        }
        keyword.push(ch);
        offset += 1;
    }
    keyword
}

fn is_newline(ch: i32) -> bool {
    matches!(
        u32::try_from(ch).ok().and_then(char::from_u32),
        Some('\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
    )
}

fn evaluate(tokens: &[(i32, String)], symbols: &BTreeSet<String>) -> bool {
    Expression {
        tokens,
        symbols,
        position: 0,
    }
    .parse_or()
}

struct Expression<'a> {
    tokens: &'a [(i32, String)],
    symbols: &'a BTreeSet<String>,
    position: usize,
}

impl Expression<'_> {
    fn peek(&self) -> Option<i32> {
        self.tokens.get(self.position).map(|(kind, _)| *kind)
    }

    fn consume(&mut self) -> Option<(i32, &str)> {
        let (kind, text) = self.tokens.get(self.position)?;
        self.position += 1;
        Some((*kind, text))
    }

    fn parse_or(&mut self) -> bool {
        let mut value = self.parse_and();
        while self.peek() == Some(OP_OR) {
            let _ = self.consume();
            value = self.parse_and() || value;
        }
        value
    }

    fn parse_and(&mut self) -> bool {
        let mut value = self.parse_eq();
        while self.peek() == Some(OP_AND) {
            let _ = self.consume();
            value = self.parse_eq() && value;
        }
        value
    }

    fn parse_eq(&mut self) -> bool {
        let value = self.parse_unary();
        match self.peek() {
            Some(OP_EQ) => {
                let _ = self.consume();
                value == self.parse_unary()
            }
            Some(OP_NE) => {
                let _ = self.consume();
                value != self.parse_unary()
            }
            _ => value,
        }
    }

    fn parse_unary(&mut self) -> bool {
        if self.peek() == Some(BANG) {
            let _ = self.consume();
            return !self.parse_unary();
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> bool {
        match self.peek() {
            Some(TRUE) => {
                let _ = self.consume();
                true
            }
            Some(FALSE) => {
                let _ = self.consume();
                false
            }
            Some(CONDITIONAL_SYMBOL) => {
                let position = self.position;
                let _ = self.consume();
                self.tokens
                    .get(position)
                    .is_some_and(|(_, symbol)| self.symbols.contains(symbol))
            }
            Some(OPEN_PARENS) => {
                let _ = self.consume();
                let value = self.parse_or();
                if self.peek() == Some(CLOSE_PARENS) {
                    let _ = self.consume();
                }
                value
            }
            _ => false,
        }
    }
}
