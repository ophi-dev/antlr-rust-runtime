lexer grammar L;

options { caseInsensitive = true; }

WS:             [ \t\r\n] -> skip;

SIMPLE_TOKEN:           'and';
TOKEN_WITH_SPACES:      'as' 'd' 'f';
TOKEN_WITH_DIGITS:      'INT64';
TOKEN_WITH_UNDERSCORE:  'TOKEN_WITH_UNDERSCORE';
BOOL:                   'true' | 'FALSE';
SPECIAL:                '==';
SET:                    [a-z0-9]+;
RANGE:                  ('а'..'я')+;