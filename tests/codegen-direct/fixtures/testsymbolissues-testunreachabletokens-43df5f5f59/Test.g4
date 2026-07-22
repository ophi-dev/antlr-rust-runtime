lexer grammar Test;
TOKEN1: 'as' 'df' | 'qwer';
TOKEN2: [0-9];
TOKEN3: 'asdf';
TOKEN4: 'q' 'w' 'e' 'r' | A;
TOKEN5: 'aaaa';
TOKEN6: 'asdf';
TOKEN7: 'qwer'+;
TOKEN8: 'a' 'b' | 'b' | 'a' 'b';
fragment
TOKEN9: 'asdf' | 'qwer' | 'qwer';
TOKEN10: '\r\n' | '\r\n';
TOKEN11: '\r\n';

mode MODE1;
TOKEN12: 'asdf';

fragment A: 'A';