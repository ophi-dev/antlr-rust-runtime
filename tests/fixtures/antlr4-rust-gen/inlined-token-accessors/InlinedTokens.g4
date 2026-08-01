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
          let __view: Option<ActiveContext<__ActiveParserContext>> =
              __active_context_view(
                  &__ctx,
                  self.base.active_invocation_states(),
                  self.base.parse_tree_storage(),
                  self.base.token_store(),
              );
          $seen = __view
              .expect("active context view")
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
