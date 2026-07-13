interface Box<T> {
    readonly value: T;
    map<U>(fn: (value: T) => U): Box<U>;
}

type Maybe<T> = T | null;
