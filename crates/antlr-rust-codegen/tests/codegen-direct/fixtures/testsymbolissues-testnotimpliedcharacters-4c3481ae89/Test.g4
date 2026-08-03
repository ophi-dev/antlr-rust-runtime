lexer grammar Test;
TOKEN1: 'A'..'g';
TOKEN2: [C-m];
TOKEN3: [А-я]; // OK since range does not contain intermediate characters
TOKEN4: '\u0100'..'\u1fff'; // OK since range borders are unicode characters