for (const value of values) {
    consume(value);
}

class Accessors {
    get value(): number { return 1; }
    set value(next: number) { consume(next); }
}
