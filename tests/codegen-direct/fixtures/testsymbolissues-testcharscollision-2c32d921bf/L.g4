lexer grammar L;
TOKEN_RANGE:      [aa-f];
TOKEN_RANGE_2:    [A-FD-J];
TOKEN_RANGE_3:    'Z' | 'K'..'R' | 'O'..'V';
TOKEN_RANGE_4:    'g'..'l' | [g-l];
TOKEN_RANGE_WITHOUT_COLLISION: '_' | [a-zA-Z];
TOKEN_RANGE_WITH_ESCAPED_CHARS: [\n-\r] | '\n'..'\r';