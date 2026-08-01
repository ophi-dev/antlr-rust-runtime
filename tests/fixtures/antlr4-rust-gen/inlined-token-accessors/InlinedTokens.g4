grammar InlinedTokens;

start
    : expr EOF
    ;

recovered
    : ID '=' ID EOF
    ;

active returns [String seen]
    : ID '='
      {
          // This is the same live view built by __active_context_view; spelling
          // its lifetime in a raw action would be parsed as a character token.
          $seen = ActiveContext {
              __node: __GeneratedRuleContext::Active {
                  context: &__ctx,
                  storage: self.base.parse_tree_storage(),
                  tokens: self.base.token_store(),
              },
              __invocation_states: Some(self.base.active_invocation_states()),
              __state: std::marker::PhantomData::<__ActiveParserContext>,
              seen: String::new(),
          }
          .direct_terminals()
          .map(|terminal| terminal.to_string())
          .collect::<Vec<_>>()
          .join(",");
      }
      ID EOF
    ;

collision
    : DIRECT DIRECT ID EOF
    ;

expr
    : assign
    | binop
    | prim
    ;

assign
    : expr '=' expr
    ;

binop
    : expr ('+' | '-') expr
    ;

prim
    : ID
    ;

DIRECT
    : 'direct'
    ;

ID
    : [a-z]+
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
