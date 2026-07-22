lexer grammar L;
options { caseInsensitive=true; }
FullWidthLetter
    : '\u00c0'..'\u00d6' // ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖ
    | '\u00f8'..'\u00ff' // øùúûüýþÿ
    ;