lexer grammar L;
options { caseInsensitive = true; }
ENGLISH_TOKEN:   [a-z]+;
GERMAN_TOKEN:    [äéöüß]+;
FRENCH_TOKEN:    [àâæ-ëîïôœùûüÿ]+;
CROATIAN_TOKEN:  [ćčđšž]+;
ITALIAN_TOKEN:   [àèéìòù]+;
SPANISH_TOKEN:   [áéíñóúü¡¿]+;
GREEK_TOKEN:     [α-ω]+;
RUSSIAN_TOKEN:   [а-я]+;
WS:              [ ]+ -> skip;