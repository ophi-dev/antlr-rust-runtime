grammar T;
	 s : e EOF;
	 e :<assoc=right> e '*' e
	   |<assoc=right> e '+' e
	   |<assoc=right> e '?' e ':' e
	   |<assoc=right> e '=' e
	   | ID
	   ;
	 ID : 'a'..'z'+ ;
	 WS : (' '|'\n') -> skip ;