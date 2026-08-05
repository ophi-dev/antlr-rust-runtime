#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Document {
    items: Vec<Item>,
}

impl Document {
    pub(crate) const fn new(items: Vec<Item>) -> Self {
        Self { items }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn into_items(self) -> Vec<Item> {
        self.items
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Item {
    Assignment(Assignment),
    Table(TableHeader),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assignment {
    key: Key,
    value: Value,
}

impl Assignment {
    pub(crate) const fn new(key: Key, value: Value) -> Self {
        Self { key, value }
    }

    pub const fn key(&self) -> &Key {
        &self.key
    }

    pub const fn value(&self) -> &Value {
        &self.value
    }

    pub fn into_parts(self) -> (Key, Value) {
        (self.key, self.value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableHeader {
    key: Key,
    array: bool,
}

impl TableHeader {
    pub(crate) const fn new(key: Key, array: bool) -> Self {
        Self { key, array }
    }

    pub const fn key(&self) -> &Key {
        &self.key
    }

    pub const fn is_array(&self) -> bool {
        self.array
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Key {
    segments: Vec<String>,
}

impl Key {
    pub(crate) const fn new(segments: Vec<String>) -> Self {
        Self { segments }
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn as_single(&self) -> Option<&str> {
        let [segment] = self.segments.as_slice() else {
            return None;
        };
        Some(segment)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    String(String),
    Integer(i64),
    Float(String),
    Boolean(bool),
    DateTime(String),
    Array(Vec<Self>),
    InlineTable(Vec<Assignment>),
}
