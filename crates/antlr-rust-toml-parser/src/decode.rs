use std::fmt;
use std::sync::{Arc, Mutex};

use antlr4_runtime::{ErrorListener, Parser as _, Recognizer, SyntaxErrorEvent};

use crate::ast::{Assignment, Document, Item, Key, TableHeader, Value};
use crate::generated::toml_lexer::TomlLexer;
use crate::generated::toml_parser::{
    ArrayContext, ArrayTableContext, BoolContext, DateTimeContext, FloatingPointContext,
    InlineTableContext, InlineTableKeyvalsNonEmptyContext, IntegerContext, KeyContext,
    KeyValueContext, SimpleKeyContext, StandardTableContext, StringContext, TomlParser,
    TomlValidatedListener, ValidatedTreeContext, parse_with_parser,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    line: Option<usize>,
    column: Option<usize>,
    message: String,
}

impl Error {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self {
            line: None,
            column: None,
            message: message.into(),
        }
    }

    fn syntax(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            column: Some(column),
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => {
                write!(
                    formatter,
                    "TOML syntax error at {line}:{column}: {}",
                    self.message
                )
            }
            _ => write!(formatter, "invalid TOML: {}", self.message),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, Default)]
struct DiagnosticCollector(Arc<Mutex<Vec<OwnedDiagnostic>>>);

#[derive(Clone, Debug)]
struct OwnedDiagnostic {
    line: usize,
    column: usize,
    message: String,
}

impl DiagnosticCollector {
    fn take(&self) -> Vec<OwnedDiagnostic> {
        std::mem::take(
            &mut *self
                .0
                .lock()
                .expect("TOML diagnostic collector mutex poisoned"),
        )
    }
}

impl<R> ErrorListener<R> for DiagnosticCollector
where
    R: Recognizer + ?Sized,
{
    fn syntax_error(&mut self, _recognizer: &R, event: &SyntaxErrorEvent<'_>) {
        self.0
            .lock()
            .expect("TOML diagnostic collector mutex poisoned")
            .push(OwnedDiagnostic {
                line: event.line,
                column: event.column,
                message: event.message.to_owned(),
            });
    }
}

pub fn parse(input: &str) -> Result<Document, Error> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let lexer_diagnostics = DiagnosticCollector::default();
    let lexer_listener = lexer_diagnostics.clone();
    let parser_diagnostics = DiagnosticCollector::default();
    let parser_listener = parser_diagnostics.clone();
    let mut output = parse_with_parser(
        input,
        move |input| {
            let mut lexer = TomlLexer::new(input);
            lexer.remove_error_listeners();
            lexer.add_error_listener(lexer_listener);
            lexer
        },
        move |parser: &mut TomlParser<_>| {
            parser.remove_error_listeners();
            parser.add_error_listener(parser_listener);
            parser.document()
        },
    )
    .map_err(|error| {
        if let Some(diagnostic) = lexer_diagnostics.take().into_iter().next() {
            return Error::syntax(diagnostic.line, diagnostic.column, diagnostic.message);
        }
        if let Some(diagnostic) = parser_diagnostics.take().into_iter().next() {
            return Error::syntax(diagnostic.line, diagnostic.column, diagnostic.message);
        }
        Error::invalid(format!("recognition failed: {error}"))
    })?;

    let drained_lexer_error = output
        .parser
        .token_stream_mut()
        .drain_source_errors()
        .into_iter()
        .next();
    let lexer_error = lexer_diagnostics.take().into_iter().next().map_or_else(
        || drained_lexer_error.map(|error| Error::syntax(error.line, error.column, error.message)),
        |error| Some(Error::syntax(error.line, error.column, error.message)),
    );
    let parser_error_count = output.parser.number_of_syntax_errors();
    let reported = parser_diagnostics.take();
    let tree = output.validate().map_err(|validation_error| {
        if let Some(error) = lexer_error {
            return error;
        }
        reported.into_iter().next().map_or_else(
            || {
                if parser_error_count == 0 {
                    Error::invalid(validation_error.to_string())
                } else {
                    Error::invalid(format!(
                        "parser reported {parser_error_count} syntax error(s)"
                    ))
                }
            },
            |error| Error::syntax(error.line, error.column, error.message),
        )
    })?;
    let mut builder = DocumentBuilder::default();
    builder.walk(tree.tree())?;
    builder.finish()
}

#[derive(Debug, Default)]
struct DocumentBuilder {
    items: Vec<Item>,
    key_segments: Vec<String>,
    key_marks: Vec<usize>,
    keys: Vec<Key>,
    values: Vec<Value>,
    array_marks: Vec<usize>,
    inline_assignments: Vec<Assignment>,
    inline_marks: Vec<usize>,
}

impl DocumentBuilder {
    fn finish(self) -> Result<Document, Error> {
        // Validated CST traversal guarantees balance; retain these checks to
        // catch regressions in the listener's own stack bookkeeping.
        if !self.key_segments.is_empty()
            || !self.key_marks.is_empty()
            || !self.keys.is_empty()
            || !self.values.is_empty()
            || !self.array_marks.is_empty()
            || !self.inline_assignments.is_empty()
            || !self.inline_marks.is_empty()
        {
            return Err(Error::invalid(
                "typed TOML walk ended with an incomplete value",
            ));
        }
        Ok(Document::new(self.items))
    }

    fn pop_key(&mut self, owner: &str) -> Result<Key, Error> {
        self.keys
            .pop()
            .ok_or_else(|| Error::invalid(format!("{owner} is missing its TOML key")))
    }

    fn pop_value(&mut self, owner: &str) -> Result<Value, Error> {
        self.values
            .pop()
            .ok_or_else(|| Error::invalid(format!("{owner} is missing its TOML value")))
    }
}

impl TomlValidatedListener<Error> for DocumentBuilder {
    fn enter_key(&mut self, _context: &KeyContext<'_, ValidatedTreeContext>) -> Result<(), Error> {
        self.key_marks.push(self.key_segments.len());
        Ok(())
    }

    fn exit_simple_key(
        &mut self,
        context: &SimpleKeyContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        self.key_segments.push(decode_simple_key(context)?);
        Ok(())
    }

    fn exit_key(&mut self, _context: &KeyContext<'_, ValidatedTreeContext>) -> Result<(), Error> {
        let start = self
            .key_marks
            .pop()
            .ok_or_else(|| Error::invalid("typed TOML key walk is unbalanced"))?;
        let segments = self.key_segments.drain(start..).collect();
        self.keys.push(Key::new(segments));
        Ok(())
    }

    fn exit_string(
        &mut self,
        context: &StringContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        self.values.push(Value::String(decode_string(context)?));
        Ok(())
    }

    fn exit_integer(
        &mut self,
        context: &IntegerContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        self.values.push(Value::Integer(decode_integer(context)?));
        Ok(())
    }

    fn exit_floating_point(
        &mut self,
        context: &FloatingPointContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        self.values.push(Value::Float(decode_float(context)?));
        Ok(())
    }

    fn exit_bool(&mut self, context: &BoolContext<'_, ValidatedTreeContext>) -> Result<(), Error> {
        let value = match context.boolean_token().to_string().as_str() {
            "true" => true,
            "false" => false,
            value => return Err(Error::invalid(format!("invalid TOML boolean {value:?}"))),
        };
        self.values.push(Value::Boolean(value));
        Ok(())
    }

    fn exit_date_time(
        &mut self,
        context: &DateTimeContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        self.values
            .push(Value::DateTime(decode_date_time(context)?));
        Ok(())
    }

    fn enter_array(
        &mut self,
        _context: &ArrayContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        self.array_marks.push(self.values.len());
        Ok(())
    }

    fn exit_array(
        &mut self,
        _context: &ArrayContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        let start = self
            .array_marks
            .pop()
            .ok_or_else(|| Error::invalid("typed TOML array walk is unbalanced"))?;
        let values = self.values.drain(start..).collect();
        self.values.push(Value::Array(values));
        Ok(())
    }

    fn enter_inline_table(
        &mut self,
        _context: &InlineTableContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        self.inline_marks.push(self.inline_assignments.len());
        Ok(())
    }

    fn exit_inline_table_keyvals_non_empty(
        &mut self,
        _context: &InlineTableKeyvalsNonEmptyContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        let value = self.pop_value("inline table entry")?;
        let key = self.pop_key("inline table entry")?;
        self.inline_assignments.push(Assignment::new(key, value));
        Ok(())
    }

    fn exit_inline_table(
        &mut self,
        _context: &InlineTableContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        let start = self
            .inline_marks
            .pop()
            .ok_or_else(|| Error::invalid("typed TOML inline-table walk is unbalanced"))?;
        // The grammar is right-recursive, so inner entries exit first.
        let values = self.inline_assignments.drain(start..).rev().collect();
        self.values.push(Value::InlineTable(values));
        Ok(())
    }

    fn exit_key_value(
        &mut self,
        _context: &KeyValueContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        let value = self.pop_value("assignment")?;
        let key = self.pop_key("assignment")?;
        self.items
            .push(Item::Assignment(Assignment::new(key, value)));
        Ok(())
    }

    fn exit_standard_table(
        &mut self,
        _context: &StandardTableContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        let key = self.pop_key("standard table")?;
        self.items.push(Item::Table(TableHeader::new(key, false)));
        Ok(())
    }

    fn exit_array_table(
        &mut self,
        _context: &ArrayTableContext<'_, ValidatedTreeContext>,
    ) -> Result<(), Error> {
        let key = self.pop_key("array table")?;
        self.items.push(Item::Table(TableHeader::new(key, true)));
        Ok(())
    }
}

fn decode_simple_key(
    context: &SimpleKeyContext<'_, ValidatedTreeContext>,
) -> Result<String, Error> {
    if let Some(unquoted) = context.unquoted_key() {
        return Ok(unquoted.unquoted_key_token().to_string());
    }
    let quoted = context
        .quoted_key()
        .ok_or_else(|| Error::invalid("TOML simple key has no value"))?;
    if let Some(token) = quoted.basic_string_token() {
        return crate::string::decode_basic(&token.to_string(), false);
    }
    let token = quoted
        .literal_string_token()
        .ok_or_else(|| Error::invalid("TOML quoted key has no string token"))?;
    crate::string::decode_literal(&token.to_string(), false)
}

fn decode_string(context: &StringContext<'_, ValidatedTreeContext>) -> Result<String, Error> {
    let (token, multiline, literal) = match (
        context.basic_string_token(),
        context.literal_string_token(),
        context.ml_basic_string_token(),
        context.ml_literal_string_token(),
    ) {
        (Some(token), _, _, _) => (token, false, false),
        (_, Some(token), _, _) => (token, false, true),
        (_, _, Some(token), _) => (token, true, false),
        (_, _, _, Some(token)) => (token, true, true),
        _ => return Err(Error::invalid("TOML string has no string token")),
    };
    if literal {
        crate::string::decode_literal(&token.to_string(), multiline)
    } else {
        crate::string::decode_basic(&token.to_string(), multiline)
    }
}

fn decode_integer(context: &IntegerContext<'_, ValidatedTreeContext>) -> Result<i64, Error> {
    let (text, radix, prefix) = if let Some(token) = context.dec_int_token() {
        (token.to_string(), 10, "")
    } else if let Some(token) = context.hex_int_token() {
        (token.to_string(), 16, "0x")
    } else if let Some(token) = context.oct_int_token() {
        (token.to_string(), 8, "0o")
    } else if let Some(token) = context.bin_int_token() {
        (token.to_string(), 2, "0b")
    } else {
        return Err(Error::invalid("TOML integer has no integer token"));
    };
    let normalized = text.chars().filter(|ch| *ch != '_').collect::<String>();
    let digits = normalized.strip_prefix(prefix).unwrap_or(&normalized);
    i64::from_str_radix(digits, radix)
        .map_err(|error| Error::invalid(format!("invalid TOML integer {text:?}: {error}")))
}

fn decode_float(context: &FloatingPointContext<'_, ValidatedTreeContext>) -> Result<String, Error> {
    context
        .float_token()
        .or_else(|| context.inf_token())
        .or_else(|| context.nan_token())
        .map(|token| token.to_string().chars().filter(|ch| *ch != '_').collect())
        .ok_or_else(|| Error::invalid("TOML float has no floating-point token"))
}

fn decode_date_time(
    context: &DateTimeContext<'_, ValidatedTreeContext>,
) -> Result<toml_datetime::Datetime, Error> {
    let value = context
        .offset_date_time_token()
        .or_else(|| context.local_date_time_token())
        .or_else(|| context.local_date_token())
        .or_else(|| context.local_time_token())
        .map(|token| token.to_string())
        .ok_or_else(|| Error::invalid("TOML date-time has no date-time token"))?;
    value
        .parse::<toml_datetime::Datetime>()
        .map_err(|error| Error::invalid(format!("invalid TOML date-time {value:?}: {error}")))
}
