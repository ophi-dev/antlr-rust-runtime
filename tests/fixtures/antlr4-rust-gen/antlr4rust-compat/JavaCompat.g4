// Reduced output from grammars-v4 java/java/Rust/transformGrammar.py at
// e756f2a2ee5565a9300666f100ba6acd874664f7.
grammar JavaCompat;

notIdentifierAssign
    : {
        !matches!(
            recog.input.la(1),
            JavaCompatParser_IDENTIFIER | JavaCompatParser_MODULE
        ) || recog.input.la(2) != JavaCompatParser_ASSIGN
    }? (IDENTIFIER | MODULE) EOF
    ;

endLookahead
    : IDENTIFIER {
        recog.input.la(1) == antlr4_runtime::TOKEN_EOF
            && recog.input.la(2) == antlr4_runtime::TOKEN_EOF
    }? EOF
    ;

recordComponentList
    : recordComponent (COMMA recordComponent)* (
        {
            _localctx.as_deref()
                .map(|ctx| {
                    let rcs = ctx.recordComponent_all();
                    let count = rcs.len();
                    (0..count).all(|i| {
                        rcs[i].IDENTIFIER().is_some()
                            && (rcs[i].ELLIPSIS().is_none() || i + 1 == count)
                    })
                })
                .unwrap_or(true)
        }? EOF
        | ASSIGN EOF
      )
    ;

recordComponent
    : IDENTIFIER ELLIPSIS?
    ;

repeatedTokens
    : IDENTIFIER IDENTIFIER {
        _localctx.as_deref()
            .map(|ctx| ctx.IDENTIFIER_all().len() == 2)
            .unwrap_or(true)
    }? EOF
    ;

commonAccessorCollisions
    : text start? child_count? rule_node? {
        _localctx.as_deref()
            .map(|ctx| {
                ctx.text().is_some()
                    && ctx.start().is_some()
                    && ctx.child_count().is_some()
                    && ctx.rule_node().is_some()
            })
            .unwrap_or(true)
    }? EOF
    ;

text: TEXT;
start: START;
child_count: CHILD_COUNT;
rule_node: RULE_NODE;

// Native equivalents used by the differential regression.
nativeNotIdentifierAssign
    : {
        !matches!(
            self.base.token_stream().la_token(1),
            IDENTIFIER | MODULE
        ) || self.base.token_stream().la_token(2) != ASSIGN
    }? (IDENTIFIER | MODULE) EOF
    ;

nativeEndLookahead
    : IDENTIFIER {
        self.base.token_stream().la_token(1) == antlr4_runtime::TOKEN_EOF
            && self.base.token_stream().la_token(2) == antlr4_runtime::TOKEN_EOF
    }? EOF
    ;

nativeRecordComponentList
    : recordComponent (COMMA recordComponent)* (
        {
            let rcs = __ctx.child_rules(
                self.base.parse_tree_storage(),
                self.base.token_store(),
                RULE_RECORD_COMPONENT,
            ).collect::<Vec<_>>();
            let count = rcs.len();
            (0..count).all(|i| {
                rcs[i].child_token(IDENTIFIER).is_some()
                    && (rcs[i].child_token(ELLIPSIS).is_none() || i + 1 == count)
            })
        }? EOF
        | ASSIGN EOF
      )
    ;

ASSIGN: '=';
COMMA: ',';
ELLIPSIS: '...';
MODULE: 'module';
TEXT: 'text';
START: 'start';
CHILD_COUNT: 'child_count';
RULE_NODE: 'rule_node';
IDENTIFIER: [a-z]+;
WS: [ \t\r\n]+ -> skip;
