grammar T;
ss[int expr] returns [int expr]
locals [int expr]
  : expr=expr EOF;
expr : '=';
