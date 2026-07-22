lexer grammar L;
options { caseInsensitive = true; }
TOKEN_RANGE:      [a-fA-F0-9];
TOKEN_RANGE_2:    'g'..'l' | 'G'..'L';
TOKEN_RANGE_3:    'm'..'q' | [M-Q];
