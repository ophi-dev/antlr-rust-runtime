grammar T;
ss : op=('=' | '+=' | expr) EOF;
expr : '=' '=';
