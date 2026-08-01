// Reduced output from grammars-v4 c/Rust/transformGrammar.py at
// e756f2a2ee5565a9300666f100ba6acd874664f7.
grammar CCompat;

assignment
    : {
        let __not_code = r#"recog.input.peek(1) _localctx.context()"#;
        // recog.output.write("not code")
        let __first = recog.input.lt(1);
        let __first_value = __first.as_ref()
            .map(|t| (t.get_token_type(), t.get_text().to_owned()));
        let __first_text = recog.input.lt(1).map(|t| t.get_text());
        let __eof = recog.input.lt(5)
            .map(|t| (t.get_token_type(), t.get_text().to_owned()));
        __first_value == Some((CCompatParser_IDENTIFIER, "left".to_owned()))
            && __first_text == Some("left")
            && !__not_code.is_empty()
            && recog.input.la(1) == CCompatParser_IDENTIFIER
            && recog.input.la(2) == CCompatParser_ASSIGN
            && recog.input.lt(0).is_none()
            && __eof.as_ref().map(|token| token.0) == Some(antlr4_runtime::TOKEN_EOF)
    }? IDENTIFIER ASSIGN IDENTIFIER EOF
    ;

history
    : IDENTIFIER IDENTIFIER {
        let mut __bk: isize = 1;
        let (__last_type, __last_text) = recog.input.lt(-__bk)
            .map(|t| (t.get_token_type(), t.get_text().to_owned()))
            .unwrap_or((-1, String::new()));
        __bk += 1;
        let __first_text = recog.input.lt(-__bk)
            .map(|t| t.get_text().to_owned())
            .unwrap_or_default();
        __last_type == CCompatParser_IDENTIFIER
            && __last_text == "second"
            && __first_text == "first"
            && recog.input.lt(-3).is_none()
    }? EOF
    ;

inlineAction
    : IDENTIFIER {
        let __ek: isize = 1;
        let __text = recog.input.lt(-__ek)
            .map(|t| t.get_text().to_owned())
            .unwrap_or_default();
        assert_eq!(__text, "action");
    } EOF
    ;

// Native equivalents used by the differential regression.
nativeAssignment
    : {
        let __first = self.base.token_stream().lt(1);
        let __first_value = __first.as_ref()
            .map(|t| (t.token_type(), t.text_or_empty().to_owned()));
        let __first_text = self.base.token_stream().lt(1).map(|t| t.text_or_empty());
        let __eof = self.base.token_stream().lt(4)
            .map(|t| (t.token_type(), t.text_or_empty().to_owned()));
        __first_value == Some((IDENTIFIER, "left".to_owned()))
            && __first_text == Some("left")
            && self.base.token_stream().la_token(1) == IDENTIFIER
            && self.base.token_stream().la_token(2) == ASSIGN
            && self.base.token_stream().lt(0).is_none()
            && __eof.as_ref().map(|token| token.0) == Some(antlr4_runtime::TOKEN_EOF)
    }? IDENTIFIER ASSIGN IDENTIFIER EOF
    ;

nativeHistory
    : IDENTIFIER IDENTIFIER {
        let mut __bk: isize = 1;
        let (__last_type, __last_text) = self.base.token_stream().lt(-__bk)
            .map(|t| (t.token_type(), t.text_or_empty().to_owned()))
            .unwrap_or((-1, String::new()));
        __bk += 1;
        let __first_text = self.base.token_stream().lt(-__bk)
            .map(|t| t.text_or_empty().to_owned())
            .unwrap_or_default();
        __last_type == IDENTIFIER
            && __last_text == "second"
            && __first_text == "first"
            && self.base.token_stream().lt(-3).is_none()
    }? EOF
    ;

nativeInlineAction
    : IDENTIFIER {
        let __ek: isize = 1;
        let __text = self.base.token_stream().lt(-__ek)
            .map(|t| t.text_or_empty().to_owned())
            .unwrap_or_default();
        assert_eq!(__text, "action");
    } EOF
    ;

ASSIGN: '=';
IDENTIFIER: [a-z]+;
WS: [ \t\r\n]+ -> skip;
