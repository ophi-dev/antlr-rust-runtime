// A grammar whose inline member carries a non-default initializer. ANTLR's own
// targets honor it, so `{ enabled }?` passes on a fresh lexer and `a` lexes as
// A. Dropping the initializer would leave the slot at 0, silently rejecting
// input the source grammar accepts (issue #206 review).
lexer grammar L;

@lexer::members
{private bool enabled = true;
}

A : { enabled }? 'a' ;
B : 'b' ;
