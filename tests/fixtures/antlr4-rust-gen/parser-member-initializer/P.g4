// Parser-side counterpart of `member-initializer`: a combined grammar whose
// parser and lexer each declare an independent same-named member with different
// kinds and initial values. Both recognizers must see their own declared value
// (issue #206 review).
grammar P;

@parser::members {
    private int level = 2;
}

@lexer::members {
    private bool level = true;
}

s : { level == 2 }? A EOF ;

A : { level }? 'a' ;
